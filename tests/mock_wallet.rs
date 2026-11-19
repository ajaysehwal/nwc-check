use std::time::Duration;

use nostr::nips::nip47::NostrWalletConnectUri;
use nostr::nips::{nip04, nip44};
use nostr_sdk::prelude::{
	Client, ClientNotification, Event, EventBuilder, Filter, FinalizeEvent, Keys, Kind, MockRelay,
	StreamExt, Tag, Timestamp,
};
use nwc_check::check::{Outcome, Status};
use nwc_check::checks::{self, Context};
use nwc_check::wallet::Wallet;
use serde_json::{Value, json};

const METHODS: [&str; 2] = ["get_info", "get_balance"];

#[derive(Clone, Copy, Default)]
struct Bugs {
	answer_expired: bool,
	wrong_error_code: bool,
	skip_p_tag: bool,
}

struct MockWallet {
	keys: Keys,
	client_keys: Keys,
	client: Client,
	bugs: Bugs,
}

impl MockWallet {
	async fn start(relay: &MockRelay, bugs: Bugs) -> NostrWalletConnectUri {
		let url = relay.url().await;
		let client = Client::default();
		client.add_relay(url.as_str()).await.unwrap();
		client.connect().and_wait(Duration::from_secs(2)).await;

		let wallet = Self { keys: Keys::generate(), client_keys: Keys::generate(), client, bugs };
		let info = EventBuilder::new(Kind::WalletConnectInfo, METHODS.join(" "))
			.tag(Tag::parse(["encryption", "nip44_v2 nip04"]).unwrap())
			.finalize(&wallet.keys)
			.unwrap();
		wallet.client.send_event(&info).await.unwrap();

		let uri = NostrWalletConnectUri::new(
			wallet.keys.public_key(),
			vec![url],
			wallet.client_keys.secret_key().clone(),
			None,
		);

		let mut notifications = wallet.client.notifications();
		let requests =
			Filter::new().kind(Kind::WalletConnectRequest).pubkey(wallet.keys.public_key());
		wallet.client.subscribe(requests).await.unwrap();
		tokio::spawn(async move {
			while let Some(notification) = notifications.next().await {
				if let ClientNotification::Event { event, .. } = notification {
					wallet.handle(&event).await;
				}
			}
		});
		uri
	}

	async fn handle(&self, request: &Event) {
		let expired = request.tags.expiration().is_some_and(|at| at < Timestamp::now());
		if expired && !self.bugs.answer_expired {
			return;
		}

		let tag = request.tags.iter().find(|tag| tag.kind() == "encryption").and_then(Tag::content);
		let nip44 = match tag {
			None | Some("nip04") => false,
			Some("nip44_v2") => true,
			Some(_) => {
				return self
					.reply(request, true, "get_info", error("UNSUPPORTED_ENCRYPTION"))
					.await;
			}
		};
		if request.pubkey != self.client_keys.public_key() {
			return self.reply(request, nip44, "get_info", error("UNAUTHORIZED")).await;
		}

		let secret = self.keys.secret_key();
		let plaintext = if nip44 {
			nip44::decrypt(secret, &request.pubkey, &request.content).unwrap()
		} else {
			nip04::decrypt(secret, &request.pubkey, &request.content).unwrap()
		};
		let body: Value = serde_json::from_str(&plaintext).unwrap();
		let method = body["method"].as_str().unwrap_or_default().to_owned();

		let outcome = match method.as_str() {
			"get_info" => Ok(json!({ "alias": "mock", "network": "regtest", "methods": METHODS })),
			"get_balance" => Ok(json!({ "balance": 21_000 })),
			_ if self.bugs.wrong_error_code => error("OTHER"),
			_ => error("NOT_IMPLEMENTED"),
		};
		self.reply(request, nip44, &method, outcome).await;
	}

	async fn reply(
		&self,
		request: &Event,
		nip44: bool,
		method: &str,
		outcome: Result<Value, Value>,
	) {
		let body = match outcome {
			Ok(result) => json!({ "result_type": method, "result": result, "error": null }),
			Err(error) => json!({ "result_type": method, "error": error }),
		}
		.to_string();
		let secret = self.keys.secret_key();
		let content = if nip44 {
			nip44::encrypt(secret, &request.pubkey, body, nip44::Version::V2).unwrap()
		} else {
			nip04::encrypt(secret, &request.pubkey, body).unwrap()
		};

		let mut builder =
			EventBuilder::new(Kind::WalletConnectResponse, content).tag(Tag::event(request.id));
		if !self.bugs.skip_p_tag {
			builder = builder.tag(Tag::public_key(request.pubkey));
		}
		self.client.send_event(&builder.finalize(&self.keys).unwrap()).await.unwrap();
	}
}

fn error(code: &str) -> Result<Value, Value> {
	Err(json!({ "code": code, "message": format!("mock {code}") }))
}

async fn run_checks(bugs: Bugs) -> Vec<Outcome> {
	let relay = MockRelay::run().await.unwrap();
	let uri = MockWallet::start(&relay, bugs).await;
	let wallet = Wallet::connect(uri, Duration::from_secs(3)).await.unwrap();
	let ctx = Context::load(wallet, Some("regtest".to_owned())).await.unwrap();
	checks::run(&ctx).await
}

fn status(outcomes: &[Outcome], id: &str) -> Status {
	outcomes.iter().find(|outcome| outcome.id == id).map(|outcome| outcome.status).unwrap()
}

#[tokio::test]
async fn compliant_wallet_has_no_failures() {
	let outcomes = run_checks(Bugs::default()).await;
	let problems: Vec<&Outcome> = outcomes
		.iter()
		.filter(|outcome| matches!(outcome.status, Status::Fail | Status::Warn))
		.collect();
	assert!(problems.is_empty(), "{problems:#?}");
	assert_eq!(outcomes.len(), checks::catalog().len());
}

#[tokio::test]
async fn buggy_wallet_is_caught() {
	let bugs = Bugs { answer_expired: true, wrong_error_code: true, skip_p_tag: true };
	let outcomes = run_checks(bugs).await;
	assert_eq!(status(&outcomes, "ERR-01"), Status::Warn);
	assert_eq!(status(&outcomes, "RESP-04"), Status::Warn);
	assert!(matches!(status(&outcomes, "SAFE-01"), Status::Fail | Status::Skip));
}
