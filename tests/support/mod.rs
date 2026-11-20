use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use futures::FutureExt;
use futures::future::BoxFuture;

use nostr::nips::nip47::NostrWalletConnectUri;
use nostr::nips::{nip04, nip44};
use nostr_sdk::prelude::{
	Client, ClientNotification, Event, EventBuilder, Filter, FinalizeEvent, Keys, Kind, MockRelay,
	StreamExt, Tag, Timestamp,
};
use nwc_check::counterparty::{Counterparty, Invoice, NodeInfo, payment_hash, to_hex};
use serde_json::{Value, json};

const METHODS: [&str; 4] = ["get_info", "get_balance", "make_invoice", "pay_invoice"];

#[derive(Clone, Copy, Default)]
pub struct Bugs {
	pub answer_expired: bool,
	pub wrong_error_code: bool,
	pub skip_p_tag: bool,
	pub wrong_preimage: bool,
}

#[derive(Clone, Default)]
pub struct MockNetwork {
	invoices: Arc<Mutex<HashMap<String, MockInvoice>>>,
}

#[derive(Clone, Copy)]
struct MockInvoice {
	preimage: [u8; 32],
	paid: bool,
}

impl MockNetwork {
	fn create(&self) -> Invoice {
		let preimage = Keys::generate().secret_key().to_secret_bytes();
		let payment_hash = payment_hash(&preimage);
		let bolt11 = format!("lnmock{}", to_hex(&payment_hash));
		self.invoices.lock().unwrap().insert(bolt11.clone(), MockInvoice { preimage, paid: false });
		Invoice { bolt11, payment_hash }
	}

	fn pay(&self, bolt11: &str) -> Result<[u8; 32]> {
		let mut invoices = self.invoices.lock().unwrap();
		let invoice = invoices.get_mut(bolt11).ok_or_else(|| anyhow!("unknown invoice"))?;
		if invoice.paid {
			bail!("invoice already paid");
		}
		invoice.paid = true;
		Ok(invoice.preimage)
	}

	fn is_paid(&self, payment_hash: &[u8; 32]) -> bool {
		let bolt11 = format!("lnmock{}", to_hex(payment_hash));
		self.invoices.lock().unwrap().get(&bolt11).is_some_and(|invoice| invoice.paid)
	}
}

pub struct MockCounterparty(pub MockNetwork);

impl Counterparty for MockCounterparty {
	fn info(&self) -> BoxFuture<'_, Result<NodeInfo>> {
		async { Ok(NodeInfo { pubkey: "mock".to_owned(), network: "regtest".to_owned() }) }.boxed()
	}

	fn create_invoice<'a>(&'a self, _: u64, _: &'a str) -> BoxFuture<'a, Result<Invoice>> {
		async { Ok(self.0.create()) }.boxed()
	}

	fn is_settled<'a>(&'a self, payment_hash: &'a [u8; 32]) -> BoxFuture<'a, Result<bool>> {
		async { Ok(self.0.is_paid(payment_hash)) }.boxed()
	}

	fn pay<'a>(&'a self, bolt11: &'a str) -> BoxFuture<'a, Result<()>> {
		async { self.0.pay(bolt11).map(|_| ()) }.boxed()
	}
}

pub struct MockWallet {
	keys: Keys,
	client_keys: Keys,
	client: Client,
	bugs: Bugs,
	network: MockNetwork,
}

impl MockWallet {
	pub async fn start(
		relay: &MockRelay,
		bugs: Bugs,
		network: MockNetwork,
	) -> NostrWalletConnectUri {
		let url = relay.url().await;
		let client = Client::default();
		client.add_relay(url.as_str()).await.unwrap();
		client.connect().and_wait(Duration::from_secs(2)).await;

		let wallet =
			Self { keys: Keys::generate(), client_keys: Keys::generate(), client, bugs, network };
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
			"make_invoice" => {
				let invoice = self.network.create();
				Ok(
					json!({ "invoice": invoice.bolt11, "payment_hash": to_hex(&invoice.payment_hash) }),
				)
			}
			"pay_invoice" => self.pay(body["params"]["invoice"].as_str().unwrap_or_default()),
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

impl MockWallet {
	fn pay(&self, bolt11: &str) -> Result<Value, Value> {
		let preimage = self.network.pay(bolt11).map_err(|_| error_value("PAYMENT_FAILED"))?;
		let preimage = if self.bugs.wrong_preimage { [0; 32] } else { preimage };
		Ok(json!({ "preimage": to_hex(&preimage) }))
	}
}

fn error_value(code: &str) -> Value {
	json!({ "code": code, "message": format!("mock {code}") })
}

fn error(code: &str) -> Result<Value, Value> {
	Err(error_value(code))
}
