use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use futures::FutureExt;
use futures::future::BoxFuture;

use bitcoin::hashes::Hash as _;
use bitcoin::hashes::sha256::Hash as Sha256Hash;
use bitcoin::secp256k1::{Message, Secp256k1, SecretKey};
use lightning_invoice::{Currency, InvoiceBuilder, PaymentSecret};
use nostr::nips::nip47::NostrWalletConnectUri;
use nostr::nips::{nip04, nip44};
use nostr_sdk::prelude::{
	Client, ClientNotification, Event, EventBuilder, EventId, Filter, FinalizeEvent, Keys, Kind,
	LocalRelay, RelayMessage, StreamExt, Tag, Timestamp,
};
use nwc_check::counterparty::{
	Counterparty, Invoice, NodeInfo, PayError, from_hex, sha256, to_hex,
};
use serde_json::{Value, json};

const FEE_MSAT: u64 = 1000;

const METHODS: [&str; 11] = [
	"get_info",
	"get_balance",
	"make_invoice",
	"lookup_invoice",
	"pay_invoice",
	"pay_keysend",
	"multi_pay_invoice",
	"multi_pay_keysend",
	"list_transactions",
	"get_budget",
	"create_connection",
];

#[derive(Clone, Copy, Default)]
pub struct Bugs {
	pub answer_expired: bool,
	pub wrong_error_code: bool,
	pub skip_p_tag: bool,
	pub skip_e_tag: bool,
	pub wrong_preimage: bool,
	pub wrong_amount: bool,
	pub ignore_description_hash: bool,
	pub lookup_by_hash_only: bool,
	pub never_settles: bool,
	pub unknown_invoice_error: bool,
	pub pays_duplicates: bool,
	pub racy_budget: bool,
	pub no_fees_paid: bool,
	pub ignores_amount: bool,
	pub hidden_fee: bool,
	pub ignores_permissions: bool,
	pub budget_missing_from_get_info: bool,
	pub budget_without_total: bool,
	pub renews_at_missing: bool,
	pub budget_ignores_fees: bool,
	pub pays_amount_param: bool,
	pub grants_any_connection: bool,
	pub budget_only_on_pay_invoice: bool,
	pub pays_stale: bool,
	pub ignores_expiry: bool,
	pub no_rate_limit: bool,
	pub stalls_after_flood: bool,
}

impl Bugs {
	/// Every bug except skip_e_tag, which hides every reply from the checks.
	pub fn all() -> Self {
		Self {
			answer_expired: true,
			wrong_error_code: true,
			skip_p_tag: true,
			wrong_preimage: true,
			wrong_amount: true,
			ignore_description_hash: true,
			lookup_by_hash_only: true,
			never_settles: true,
			unknown_invoice_error: true,
			pays_duplicates: true,
			racy_budget: true,
			no_fees_paid: true,
			ignores_amount: true,
			hidden_fee: true,
			ignores_permissions: true,
			budget_missing_from_get_info: true,
			budget_without_total: true,
			renews_at_missing: true,
			budget_ignores_fees: true,
			pays_amount_param: true,
			grants_any_connection: true,
			budget_only_on_pay_invoice: true,
			pays_stale: true,
			ignores_expiry: true,
			no_rate_limit: true,
			stalls_after_flood: true,
			..Self::default()
		}
	}
}

pub enum Memo {
	Text(String),
	Hash([u8; 32]),
}

#[derive(Clone)]
pub struct MockNetwork {
	chain: &'static str,
	node_key: SecretKey,
	invoices: Arc<Mutex<HashMap<String, MockInvoice>>>,
}

#[derive(Clone)]
struct MockInvoice {
	bolt11: String,
	preimage: [u8; 32],
	amount_msat: u64,
	paid: bool,
}

impl Default for MockNetwork {
	fn default() -> Self {
		Self::new("regtest")
	}
}

impl MockNetwork {
	pub fn new(chain: &'static str) -> Self {
		let node_key = SecretKey::from_slice(&random_bytes()).unwrap();
		Self { chain, node_key, invoices: Arc::default() }
	}

	pub fn moved_msat(&self) -> u64 {
		let invoices = self.invoices.lock().unwrap();
		invoices.values().filter(|invoice| invoice.paid).map(|invoice| invoice.amount_msat).sum()
	}

	fn create(&self, amount_msat: u64, memo: Memo, expiry: Duration) -> Invoice {
		let preimage = random_bytes();
		let payment_hash = sha256(&preimage);
		let currency = if self.chain == "mainnet" { Currency::Bitcoin } else { Currency::Regtest };
		let builder = InvoiceBuilder::new(currency);
		let builder = match memo {
			Memo::Text(text) => builder.description(text),
			Memo::Hash(hash) => builder.description_hash(Sha256Hash::from_byte_array(hash)),
		};
		let builder = builder
			.payment_hash(Sha256Hash::from_byte_array(payment_hash))
			.payment_secret(PaymentSecret(random_bytes()))
			.expiry_time(expiry)
			.duration_since_epoch(SystemTime::now().duration_since(UNIX_EPOCH).unwrap())
			.min_final_cltv_expiry_delta(144);
		let sign =
			|message: &Message| Secp256k1::new().sign_ecdsa_recoverable(message, &self.node_key);
		let signed = if amount_msat > 0 {
			builder.amount_milli_satoshis(amount_msat).build_signed(sign)
		} else {
			builder.build_signed(sign)
		};
		let bolt11 = signed.unwrap().to_string();
		let invoice = MockInvoice { bolt11: bolt11.clone(), preimage, amount_msat, paid: false };
		self.invoices.lock().unwrap().insert(to_hex(&payment_hash), invoice);
		Invoice { bolt11, payment_hash }
	}

	fn amount_msat(&self, bolt11: &str) -> Option<u64> {
		let invoices = self.invoices.lock().unwrap();
		invoices
			.values()
			.find(|invoice| invoice.bolt11 == bolt11)
			.map(|invoice| invoice.amount_msat)
	}

	fn pay(&self, bolt11: &str, repeat: bool, amount_msat: Option<u64>) -> Result<[u8; 32]> {
		let mut invoices = self.invoices.lock().unwrap();
		let invoice = invoices
			.values_mut()
			.find(|invoice| invoice.bolt11 == bolt11)
			.ok_or_else(|| anyhow!("unknown invoice"))?;
		if invoice.paid && !repeat {
			bail!("invoice already paid");
		}
		match amount_msat {
			Some(amount_msat) => invoice.amount_msat = amount_msat,
			None if invoice.amount_msat == 0 => bail!("amountless invoice"),
			None => {}
		}
		invoice.paid = true;
		Ok(invoice.preimage)
	}

	/// Records a keysend as an invoice that was paid on arrival, as LND does.
	fn keysend(&self, preimage: [u8; 32], amount_msat: u64) {
		let invoice = MockInvoice { bolt11: String::new(), preimage, amount_msat, paid: true };
		self.invoices.lock().unwrap().insert(to_hex(&sha256(&preimage)), invoice);
	}

	fn find(&self, payment_hash: Option<&str>, bolt11: Option<&str>) -> Option<MockInvoice> {
		let invoices = self.invoices.lock().unwrap();
		match (payment_hash, bolt11) {
			(Some(hash), _) => invoices.get(hash).cloned(),
			(None, Some(bolt11)) => invoices.values().find(|i| i.bolt11 == bolt11).cloned(),
			(None, None) => None,
		}
	}
}

fn random_bytes() -> [u8; 32] {
	Keys::generate().secret_key().to_secret_bytes()
}

pub struct MockCounterparty {
	pub network: MockNetwork,
	pub broke: bool,
}

impl Counterparty for MockCounterparty {
	fn info(&self) -> BoxFuture<'_, Result<NodeInfo>> {
		let network = self.network.chain.to_owned();
		async { Ok(NodeInfo { pubkey: "mock".to_owned(), network }) }.boxed()
	}

	fn create_invoice<'a>(
		&'a self,
		amount_msat: u64,
		memo: &'a str,
	) -> BoxFuture<'a, Result<Invoice>> {
		let invoice = self.network.create(
			amount_msat,
			Memo::Text(memo.to_owned()),
			Duration::from_secs(3600),
		);
		async move { Ok(invoice) }.boxed()
	}

	fn paid_msat<'a>(&'a self, payment_hash: &'a [u8; 32]) -> BoxFuture<'a, Result<Option<u64>>> {
		let invoice = self.network.find(Some(&to_hex(payment_hash)), None);
		let paid = invoice.filter(|invoice| invoice.paid).map(|invoice| invoice.amount_msat);
		async move { Ok(paid) }.boxed()
	}

	fn pay<'a>(&'a self, bolt11: &'a str) -> BoxFuture<'a, Result<(), PayError>> {
		let result = if self.broke {
			Err(PayError::Unable("insufficient_balance".to_owned()))
		} else {
			self.network
				.pay(bolt11, false, None)
				.map(|_| ())
				.map_err(|error| PayError::Failed(error.to_string()))
		};
		async move { result }.boxed()
	}
}

pub struct MockConnections {
	pub uri: NostrWalletConnectUri,
	pub readonly: NostrWalletConnectUri,
}

pub struct MockWallet {
	keys: Keys,
	client_keys: Keys,
	readonly_keys: Keys,
	client: Client,
	bugs: Bugs,
	network: MockNetwork,
	seen: Mutex<HashSet<EventId>>,
	paid: Mutex<Vec<[u8; 32]>>,
	budget: Option<Mutex<Budget>>,
	balance_msat: Mutex<u64>,
	recent: Mutex<VecDeque<Instant>>,
	stalled: AtomicBool,
}

struct Budget {
	total_msat: u64,
	used_msat: u64,
}

impl Budget {
	fn fits(&self, amount_msat: u64) -> bool {
		self.used_msat + amount_msat <= self.total_msat
	}
}

impl MockWallet {
	pub async fn start(
		relay: &LocalRelay,
		bugs: Bugs,
		network: MockNetwork,
		budget_msat: Option<u64>,
	) -> MockConnections {
		let url = relay.url().await;
		let client = Client::default();
		client.add_relay(url.as_str()).await.unwrap();
		client.connect().and_wait(Duration::from_secs(2)).await;

		let wallet = Self {
			keys: Keys::generate(),
			client_keys: Keys::generate(),
			readonly_keys: Keys::generate(),
			client,
			bugs,
			network,
			seen: Mutex::default(),
			paid: Mutex::default(),
			budget: budget_msat.map(|total_msat| Mutex::new(Budget { total_msat, used_msat: 0 })),
			balance_msat: Mutex::new(100_000_000),
			recent: Mutex::default(),
			stalled: AtomicBool::new(false),
		};
		let info = EventBuilder::new(Kind::WalletConnectInfo, METHODS.join(" "))
			.tag(Tag::parse(["encryption", "nip44_v2 nip04"]).unwrap())
			.finalize(&wallet.keys)
			.unwrap();
		wallet.client.send_event(&info).await.unwrap();

		let connection = |keys: &Keys| {
			NostrWalletConnectUri::new(
				wallet.keys.public_key(),
				vec![url.clone()],
				keys.secret_key().clone(),
				None,
			)
		};
		let connections = MockConnections {
			uri: connection(&wallet.client_keys),
			readonly: connection(&wallet.readonly_keys),
		};

		let mut notifications = wallet.client.notifications();
		let requests =
			Filter::new().kind(Kind::WalletConnectRequest).pubkey(wallet.keys.public_key());
		wallet.client.subscribe(requests).await.unwrap();
		let wallet = Arc::new(wallet);
		tokio::spawn(async move {
			while let Some(notification) = notifications.next().await {
				// Raw relay messages, so duplicates of an event arrive too.
				if let ClientNotification::Message { message, .. } = notification
					&& let RelayMessage::Event { event, .. } = *message
				{
					let (wallet, event) = (wallet.clone(), event.into_owned());
					tokio::spawn(async move { wallet.handle(&event).await });
				}
			}
		});
		connections
	}

	/// Whether get_info has been asked more than 20 times in the last 3 seconds.
	fn flooded(&self) -> bool {
		let now = Instant::now();
		let mut recent = self.recent.lock().unwrap();
		recent.retain(|at| now.duration_since(*at) < Duration::from_secs(3));
		recent.push_back(now);
		recent.len() > 20
	}

	async fn handle(&self, request: &Event) {
		let expired = request.tags.expiration().is_some_and(|at| at < Timestamp::now());
		if expired && !self.bugs.answer_expired {
			return;
		}
		if !self.seen.lock().unwrap().insert(request.id) && !self.bugs.pays_duplicates {
			return;
		}
		// A request signed more than ten minutes ago is ignored, unless the wallet has no age limit.
		let stale = request.created_at < Timestamp::from_secs(Timestamp::now().as_secs() - 10 * 60);
		if stale && !self.bugs.pays_stale {
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
		let readonly = request.pubkey == self.readonly_keys.public_key();
		if request.pubkey != self.client_keys.public_key() && !readonly {
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

		if self.stalled.load(Ordering::SeqCst) {
			return;
		}
		if method == "get_info" && self.flooded() {
			if self.bugs.stalls_after_flood {
				self.stalled.store(true, Ordering::SeqCst);
				return;
			}
			if !self.bugs.no_rate_limit {
				return self.reply(request, nip44, &method, error("RATE_LIMITED")).await;
			}
		}

		let outcome = match method.as_str() {
			"pay_invoice" if readonly && !self.bugs.ignores_permissions => error("RESTRICTED"),
			"get_info" => {
				let methods: Vec<&str> = METHODS
					.into_iter()
					.filter(|m| *m != "get_budget" || !self.bugs.budget_missing_from_get_info)
					.collect();
				Ok(json!({ "alias": "mock", "network": self.network.chain, "methods": methods }))
			}
			"get_balance" => Ok(json!({ "balance": *self.balance_msat.lock().unwrap() })),
			"make_invoice" => Ok(self.make_invoice(&body["params"])),
			"lookup_invoice" => self.lookup_invoice(&body["params"]),
			"pay_invoice" => self.pay(&body["params"], true).await,
			"pay_keysend" => {
				self.keysend(&body["params"], !self.bugs.budget_only_on_pay_invoice).await
			}
			// One reply for the first item is enough for the checks that use batches.
			"multi_pay_invoice" => {
				let first = &body["params"]["invoices"][0];
				self.pay(first, !self.bugs.budget_only_on_pay_invoice).await
			}
			"multi_pay_keysend" => {
				let first = &body["params"]["keysends"][0];
				self.keysend(first, !self.bugs.budget_only_on_pay_invoice).await
			}
			"get_budget" => Ok(self.get_budget()),
			"create_connection" => self.create_connection(&body["params"]),
			"list_transactions" => Ok(self.list_transactions()),
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

		let mut builder = EventBuilder::new(Kind::WalletConnectResponse, content);
		if !self.bugs.skip_e_tag {
			builder = builder.tag(Tag::event(request.id));
		}
		if !self.bugs.skip_p_tag {
			builder = builder.tag(Tag::public_key(request.pubkey));
		}
		self.client.send_event(&builder.finalize(&self.keys).unwrap()).await.unwrap();
	}
}

impl MockWallet {
	fn make_invoice(&self, params: &Value) -> Value {
		let mut amount_msat = params["amount"].as_u64().unwrap_or(0);
		if self.bugs.wrong_amount {
			amount_msat += 1000;
		}
		let description = params["description"].as_str().unwrap_or_default().to_owned();
		let hash = params["description_hash"]
			.as_str()
			.and_then(from_hex)
			.and_then(|bytes| <[u8; 32]>::try_from(bytes).ok());
		let memo = match hash {
			Some(hash) if !self.bugs.ignore_description_hash => Memo::Hash(hash),
			_ => Memo::Text(description),
		};
		let asked = params["expiry"].as_u64().filter(|_| !self.bugs.ignores_expiry);
		let expiry = Duration::from_secs(asked.unwrap_or(3600));
		let invoice = self.network.create(amount_msat, memo, expiry);
		json!({
			"type": "incoming",
			"invoice": invoice.bolt11,
			"payment_hash": to_hex(&invoice.payment_hash),
			"amount": amount_msat,
			"state": "pending",
		})
	}

	fn lookup_invoice(&self, params: &Value) -> Result<Value, Value> {
		let bolt11 = params["invoice"].as_str().filter(|_| !self.bugs.lookup_by_hash_only);
		let not_found = if self.bugs.unknown_invoice_error { "INTERNAL" } else { "NOT_FOUND" };
		let invoice = self
			.network
			.find(params["payment_hash"].as_str(), bolt11)
			.ok_or_else(|| error_value(not_found))?;
		let settled = invoice.paid && !self.bugs.never_settles;
		Ok(json!({
			"type": "incoming",
			"invoice": invoice.bolt11,
			"payment_hash": to_hex(&sha256(&invoice.preimage)),
			"amount": invoice.amount_msat,
			"state": if settled { "settled" } else { "pending" },
		}))
	}

	fn list_transactions(&self) -> Value {
		let transactions: Vec<Value> = self
			.paid
			.lock()
			.unwrap()
			.iter()
			.map(
				|hash| json!({ "type": "outgoing", "payment_hash": to_hex(hash), "state": "settled" }),
			)
			.collect();
		json!({ "transactions": transactions })
	}

	/// Creates a connection only within this connection's own methods and remaining budget.
	fn create_connection(&self, params: &Value) -> Result<Value, Value> {
		let granted = json!({ "wallet_pubkey": Keys::generate().public_key().to_hex() });
		if self.bugs.grants_any_connection {
			return Ok(granted);
		}
		let methods = params["request_methods"].as_array().cloned().unwrap_or_default();
		let unknown = methods.iter().any(|method| {
			let method = method.as_str().unwrap_or_default();
			method == "create_connection" || !METHODS.contains(&method)
		});
		let max_amount = params["max_amount"].as_u64().unwrap_or(0);
		let over_budget = self.budget.as_ref().is_some_and(|budget| {
			let budget = budget.lock().unwrap();
			max_amount > budget.total_msat.saturating_sub(budget.used_msat)
		});
		if unknown || over_budget { Err(error_value("RESTRICTED")) } else { Ok(granted) }
	}

	fn get_budget(&self) -> Value {
		match &self.budget {
			Some(budget) => {
				let budget = budget.lock().unwrap();
				let mut result = json!({
					"used_budget": budget.used_msat,
					"total_budget": budget.total_msat,
					"renewal_period": "monthly",
					"renews_at": Timestamp::now().as_secs() + 30 * 24 * 3600,
				});
				if self.bugs.budget_without_total {
					result.as_object_mut().unwrap().remove("total_budget");
				}
				if self.bugs.renews_at_missing {
					result.as_object_mut().unwrap().remove("renews_at");
				}
				result
			}
			None => json!({}),
		}
	}

	async fn charge(&self, amount_msat: u64) -> Result<(), Value> {
		let Some(budget) = &self.budget else { return Ok(()) };
		if !self.bugs.racy_budget {
			let mut budget = budget.lock().unwrap();
			if !budget.fits(amount_msat) {
				return Err(error_value("QUOTA_EXCEEDED"));
			}
			budget.used_msat += amount_msat;
			return Ok(());
		}
		let fits = budget.lock().unwrap().fits(amount_msat);
		if !fits {
			return Err(error_value("QUOTA_EXCEEDED"));
		}
		tokio::time::sleep(Duration::from_millis(50)).await;
		budget.lock().unwrap().used_msat += amount_msat;
		Ok(())
	}

	async fn keysend(&self, params: &Value, charge: bool) -> Result<Value, Value> {
		let failed = || error_value("PAYMENT_FAILED");
		let amount_msat = params["amount"].as_u64().ok_or_else(failed)?;
		let preimage = params["preimage"]
			.as_str()
			.and_then(from_hex)
			.and_then(|bytes| <[u8; 32]>::try_from(bytes).ok())
			.unwrap_or_else(random_bytes);
		if charge {
			self.charge(amount_msat + FEE_MSAT).await?;
		}
		self.network.keysend(preimage, amount_msat);
		let mut balance = self.balance_msat.lock().unwrap();
		*balance = balance.saturating_sub(amount_msat + FEE_MSAT);
		Ok(json!({ "preimage": to_hex(&preimage), "fees_paid": FEE_MSAT }))
	}

	async fn pay(&self, params: &Value, charge: bool) -> Result<Value, Value> {
		let failed = || error_value("PAYMENT_FAILED");
		let bolt11 = params["invoice"].as_str().unwrap_or_default();
		let param = params["amount"].as_u64();
		let amount_msat = match (self.network.amount_msat(bolt11).ok_or_else(failed)?, param) {
			(0, _) if self.bugs.ignores_amount => return Err(failed()),
			(0, param) => param.ok_or_else(failed)?,
			(_, Some(param)) if self.bugs.pays_amount_param => param,
			(fixed, Some(param)) if param != fixed => return Err(error_value("OTHER")),
			(fixed, _) => fixed,
		};
		let fee = if self.bugs.budget_ignores_fees { 0 } else { FEE_MSAT };
		if charge {
			self.charge(amount_msat + fee).await?;
		}
		let preimage = self
			.network
			.pay(bolt11, self.bugs.pays_duplicates, Some(amount_msat))
			.map_err(|_| failed())?;
		let hidden = if self.bugs.hidden_fee { 1000 } else { 0 };
		let mut balance = self.balance_msat.lock().unwrap();
		*balance = balance.saturating_sub(amount_msat + FEE_MSAT + hidden);
		drop(balance);
		self.paid.lock().unwrap().push(sha256(&preimage));

		let preimage = if self.bugs.wrong_preimage { [0; 32] } else { preimage };
		let mut result = json!({ "preimage": to_hex(&preimage) });
		if !self.bugs.no_fees_paid {
			result["fees_paid"] = json!(FEE_MSAT);
		}
		Ok(result)
	}
}

fn error_value(code: &str) -> Value {
	json!({ "code": code, "message": format!("mock {code}") })
}

fn error(code: &str) -> Result<Value, Value> {
	Err(error_value(code))
}
