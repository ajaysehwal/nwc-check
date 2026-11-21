use std::fmt;
use std::pin::Pin;
use std::time::Duration;

use futures::Stream;
use nostr::nips::nip47::NostrWalletConnectUri;
use nostr::nips::{nip04, nip44};
use nostr_sdk::prelude::RelayUrl;
use nostr_sdk::prelude::{
	Client, Event, EventBuilder, EventId, Filter, FinalizeEvent, Keys, Kind, PublicKey,
	ReqExitPolicy, SecretKey, StreamExt, Tag, Timestamp,
};
use serde_json::{Value, json};

type Responses =
	Pin<Box<dyn Stream<Item = (RelayUrl, Result<Event, nostr_sdk::error::Error>)> + Send>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
	Nip44,
	Nip04,
}

impl Scheme {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Nip44 => "nip44_v2",
			Self::Nip04 => "nip04",
		}
	}
}

#[derive(Clone, Copy, Debug)]
pub enum Encryption {
	Tagged(Scheme),
	Untagged,
	Unsupported(&'static str),
}

impl Encryption {
	fn scheme(self) -> Scheme {
		match self {
			Self::Tagged(scheme) => scheme,
			Self::Untagged => Scheme::Nip04,
			Self::Unsupported(_) => Scheme::Nip44,
		}
	}

	fn tag(self) -> Option<&'static str> {
		match self {
			Self::Tagged(scheme) => Some(scheme.as_str()),
			Self::Untagged => None,
			Self::Unsupported(name) => Some(name),
		}
	}
}

#[derive(Clone, Debug)]
pub struct Request {
	method: String,
	params: Value,
	encryption: Encryption,
	expiration: Option<Timestamp>,
	signer: Option<Keys>,
}

impl Request {
	pub fn new(method: impl Into<String>, encryption: Encryption) -> Self {
		Self {
			method: method.into(),
			params: json!({}),
			encryption,
			expiration: None,
			signer: None,
		}
	}

	pub fn params(mut self, params: Value) -> Self {
		self.params = params;
		self
	}

	pub fn expiration(mut self, expiration: Timestamp) -> Self {
		self.expiration = Some(expiration);
		self
	}

	pub fn signer(mut self, signer: Keys) -> Self {
		self.signer = Some(signer);
		self
	}

	pub fn method(&self) -> &str {
		&self.method
	}
}

#[derive(Clone, Debug)]
pub struct Reply {
	pub request_id: EventId,
	pub client: PublicKey,
	pub event: Event,
	pub scheme: Scheme,
	pub body: Value,
}

impl Reply {
	pub fn result_type(&self) -> Option<&str> {
		self.body.get("result_type").and_then(Value::as_str)
	}

	pub fn error(&self) -> Option<&Value> {
		self.body.get("error").filter(|error| !error.is_null())
	}

	pub fn error_code(&self) -> Option<&str> {
		self.error().and_then(|error| error.get("code")).and_then(Value::as_str)
	}

	pub fn result(&self) -> Option<&Value> {
		self.body.get("result").filter(|result| !result.is_null())
	}
}

#[derive(Debug)]
pub enum WalletError {
	NoReply,
	Rejected(String),
	Undecryptable,
	InvalidJson(String),
	Relay(String),
}

impl fmt::Display for WalletError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NoReply => write!(f, "no reply before the timeout"),
			Self::Rejected(reason) => write!(f, "relays rejected the request: {reason}"),
			Self::Undecryptable => write!(f, "reply could not be decrypted"),
			Self::InvalidJson(error) => write!(f, "reply is not valid JSON: {error}"),
			Self::Relay(error) => write!(f, "relay error: {error}"),
		}
	}
}

impl std::error::Error for WalletError {}

fn relay_error(error: impl fmt::Display) -> WalletError {
	WalletError::Relay(error.to_string())
}

pub struct Signed {
	event: Event,
	scheme: Scheme,
	signer: Keys,
}

pub struct Redelivery {
	pub echoed: bool,
	pub reply: Option<Reply>,
}

pub struct Wallet {
	client: Client,
	uri: NostrWalletConnectUri,
	keys: Keys,
	timeout: Duration,
}

impl Wallet {
	pub async fn connect(
		uri: NostrWalletConnectUri,
		timeout: Duration,
	) -> Result<Self, WalletError> {
		let client = Client::default();
		for relay in &uri.relays {
			client.add_relay(relay.as_str()).await.map_err(relay_error)?;
		}
		client.connect().and_wait(timeout).await;
		let keys = Keys::new(uri.secret.clone());
		Ok(Self { client, uri, keys, timeout })
	}

	pub fn public_key(&self) -> PublicKey {
		self.uri.public_key
	}

	pub fn relays(&self) -> Vec<String> {
		self.uri.relays.iter().map(ToString::to_string).collect()
	}

	pub async fn info_event(&self) -> Result<Option<Event>, WalletError> {
		let filter = Filter::new().kind(Kind::WalletConnectInfo).author(self.uri.public_key);
		let events =
			self.client.fetch_events(filter).timeout(self.timeout).await.map_err(relay_error)?;
		Ok(events.into_iter().max_by_key(|event| event.created_at))
	}

	pub async fn send(&self, request: Request) -> Result<Reply, WalletError> {
		self.deliver(&self.sign(request)?).await
	}

	pub fn sign(&self, request: Request) -> Result<Signed, WalletError> {
		let signer = request.signer.clone().unwrap_or_else(|| self.keys.clone());
		let event = self.build_event(&request, &signer)?;
		Ok(Signed { event, scheme: request.encryption.scheme(), signer })
	}

	pub async fn deliver(&self, signed: &Signed) -> Result<Reply, WalletError> {
		let mut replies = self.replies(signed).await?;
		self.publish(signed).await?;
		let (_, response) = replies.next().await.ok_or(WalletError::NoReply)?;
		self.open(signed, response.map_err(relay_error)?)
	}

	pub async fn redeliver(&self, signed: &Signed) -> Result<Redelivery, WalletError> {
		let mut echoes = self
			.client
			.stream_events(Filter::new().id(signed.event.id))
			.timeout(self.timeout)
			.policy(ReqExitPolicy::WaitForEvents(1))
			.await
			.map_err(relay_error)?;
		let mut replies = self.replies(signed).await?;
		self.publish(signed).await?;
		let (echo, reply) = tokio::join!(echoes.next(), replies.next());
		let reply = match reply {
			Some((_, response)) => Some(self.open(signed, response.map_err(relay_error)?)?),
			None => None,
		};
		Ok(Redelivery { echoed: echo.is_some(), reply })
	}

	async fn replies(&self, signed: &Signed) -> Result<Responses, WalletError> {
		let filter = Filter::new()
			.kind(Kind::WalletConnectResponse)
			.author(self.uri.public_key)
			.event(signed.event.id);
		self.client
			.stream_events(filter)
			.timeout(self.timeout)
			.policy(ReqExitPolicy::WaitForEvents(1))
			.await
			.map_err(relay_error)
	}

	async fn publish(&self, signed: &Signed) -> Result<(), WalletError> {
		let output = self.client.send_event(&signed.event).await.map_err(relay_error)?;
		if output.success.is_empty() {
			let reasons: Vec<String> = output.failed.into_values().collect();
			return Err(WalletError::Rejected(reasons.join("; ")));
		}
		Ok(())
	}

	fn open(&self, signed: &Signed, response: Event) -> Result<Reply, WalletError> {
		let (scheme, plaintext) = decrypt(signed.signer.secret_key(), &response, signed.scheme)?;
		let body = serde_json::from_str(&plaintext)
			.map_err(|error| WalletError::InvalidJson(error.to_string()))?;
		Ok(Reply {
			request_id: signed.event.id,
			client: signed.signer.public_key(),
			event: response,
			scheme,
			body,
		})
	}

	pub async fn disconnect(self) {
		self.client.shutdown().await;
	}

	fn build_event(&self, request: &Request, signer: &Keys) -> Result<Event, WalletError> {
		let payload = json!({ "method": request.method, "params": request.params }).to_string();
		let wallet = &self.uri.public_key;
		let content = match request.encryption.scheme() {
			Scheme::Nip44 => {
				nip44::encrypt(signer.secret_key(), wallet, payload, nip44::Version::V2)
					.map_err(relay_error)?
			}
			Scheme::Nip04 => {
				nip04::encrypt(signer.secret_key(), wallet, payload).map_err(relay_error)?
			}
		};

		let mut builder =
			EventBuilder::new(Kind::WalletConnectRequest, content).tag(Tag::public_key(*wallet));
		if let Some(name) = request.encryption.tag() {
			builder = builder.tag(Tag::parse(["encryption", name]).map_err(relay_error)?);
		}
		if let Some(expiration) = request.expiration {
			builder = builder.tag(Tag::expiration(expiration));
		}
		builder.finalize(signer).map_err(relay_error)
	}
}

fn decrypt(
	secret: &SecretKey,
	event: &Event,
	expected: Scheme,
) -> Result<(Scheme, String), WalletError> {
	let attempt = |scheme| match scheme {
		Scheme::Nip44 => nip44::decrypt(secret, &event.pubkey, &event.content).ok(),
		Scheme::Nip04 => nip04::decrypt(secret, &event.pubkey, &event.content).ok(),
	};
	let other = if expected == Scheme::Nip44 { Scheme::Nip04 } else { Scheme::Nip44 };
	[expected, other]
		.into_iter()
		.find_map(|scheme| attempt(scheme).map(|plaintext| (scheme, plaintext)))
		.ok_or(WalletError::Undecryptable)
}
