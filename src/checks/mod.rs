mod discovery;
mod encryption;
mod errors;
mod invoices;
mod payments;
mod response;
mod safety;

use std::sync::Mutex;

use futures::StreamExt;
use futures::stream::FuturesUnordered;
use nostr_sdk::prelude::Event;

use crate::check::{Check, Outcome};
use crate::counterparty::Counterparty;
use crate::wallet::{Encryption, Reply, Request, Scheme, Wallet, WalletError};

pub const UNKNOWN_METHOD: &str = "nwc_check_unknown_method";
pub const NO_COUNTERPARTY: &str = "no counterparty node (see --lnd-url)";

pub struct Payments {
	pub node: Box<dyn Counterparty>,
	pub network: String,
	pub amount_msat: u64,
	remaining_msat: Mutex<u64>,
}

impl Payments {
	pub fn new(
		node: Box<dyn Counterparty>,
		network: String,
		amount_msat: u64,
		max_msat: u64,
	) -> Self {
		Self { node, network, amount_msat, remaining_msat: Mutex::new(max_msat) }
	}

	pub fn reserve(&self) -> Result<(), String> {
		let mut remaining = self.remaining_msat.lock().unwrap_or_else(|e| e.into_inner());
		if *remaining < self.amount_msat {
			return Err(format!("would go over --max-sats ({} sats left)", *remaining / 1000));
		}
		*remaining -= self.amount_msat;
		Ok(())
	}
}

pub struct Context {
	pub wallet: Wallet,
	pub info: Option<Event>,
	pub methods: Vec<String>,
	pub schemes: Vec<String>,
	pub expected_network: Option<String>,
	pub get_info: Result<Reply, WalletError>,
	pub unknown_method: Result<Reply, WalletError>,
	pub payments: Result<Payments, String>,
}

impl Context {
	pub async fn load(
		wallet: Wallet,
		expected_network: Option<String>,
	) -> Result<Self, WalletError> {
		let info = wallet.info_event().await?;
		let methods: Vec<String> = info
			.as_ref()
			.map(|event| event.content.split_whitespace().map(str::to_owned).collect())
			.unwrap_or_default();
		let schemes: Vec<String> = info
			.as_ref()
			.and_then(|event| event.tags.iter().find(|tag| tag.kind() == "encryption"))
			.and_then(|tag| tag.content())
			.map(|value| value.split_whitespace().map(str::to_owned).collect())
			.unwrap_or_default();

		let encryption = preferred_encryption(&schemes);
		let (get_info, unknown_method) = tokio::join!(
			wallet.send(Request::new("get_info", encryption)),
			wallet.send(Request::new(UNKNOWN_METHOD, encryption)),
		);

		Ok(Self {
			wallet,
			info,
			methods,
			schemes,
			expected_network,
			get_info,
			unknown_method,
			payments: Err(NO_COUNTERPARTY.to_owned()),
		})
	}

	pub fn with_payments(mut self, payments: Payments) -> Self {
		self.payments = match self.network() {
			None => Err("the wallet didn't report its network".to_owned()),
			Some("mainnet") => Err("the wallet is on mainnet".to_owned()),
			_ if payments.network == "mainnet" => Err("the counterparty is on mainnet".to_owned()),
			Some(network) if network != payments.network => Err(format!(
				"the wallet is on {network} but the counterparty is on {}",
				payments.network
			)),
			_ => Ok(payments),
		};
		self
	}

	pub fn block_payments(mut self, reason: impl Into<String>) -> Self {
		self.payments = Err(reason.into());
		self
	}

	pub fn network(&self) -> Option<&str> {
		self.get_info.as_ref().ok()?.result()?.get("network")?.as_str()
	}

	pub fn supports(&self, method: &str) -> bool {
		self.methods.iter().any(|m| m == method)
	}

	pub fn advertises(&self, scheme: Scheme) -> bool {
		self.schemes.iter().any(|s| s == scheme.as_str())
	}

	pub fn encryption(&self) -> Encryption {
		preferred_encryption(&self.schemes)
	}
}

fn preferred_encryption(schemes: &[String]) -> Encryption {
	if schemes.iter().any(|s| s == Scheme::Nip44.as_str()) {
		Encryption::Tagged(Scheme::Nip44)
	} else {
		Encryption::Untagged
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
	Discovery,
	Encryption,
	Responses,
	Errors,
	Safety,
	Invoices,
	Payments,
}

impl Section {
	pub const ALL: [Section; 7] = [
		Self::Discovery,
		Self::Encryption,
		Self::Responses,
		Self::Errors,
		Self::Safety,
		Self::Invoices,
		Self::Payments,
	];

	pub fn title(self) -> &'static str {
		match self {
			Self::Discovery => "Discovery",
			Self::Encryption => "Encryption",
			Self::Responses => "Responses",
			Self::Errors => "Errors",
			Self::Safety => "Safety",
			Self::Invoices => "Invoices",
			Self::Payments => "Payments",
		}
	}

	pub fn checks(self) -> &'static [&'static Check] {
		match self {
			Self::Discovery => discovery::CHECKS,
			Self::Encryption => encryption::CHECKS,
			Self::Responses => response::CHECKS,
			Self::Errors => errors::CHECKS,
			Self::Safety => safety::CHECKS,
			Self::Invoices => invoices::CHECKS,
			Self::Payments => payments::CHECKS,
		}
	}

	async fn run(self, ctx: &Context) -> Vec<Outcome> {
		match self {
			Self::Discovery => discovery::run(ctx).await,
			Self::Encryption => encryption::run(ctx).await,
			Self::Responses => response::run(ctx).await,
			Self::Errors => errors::run(ctx).await,
			Self::Safety => safety::run(ctx).await,
			Self::Invoices => invoices::run(ctx).await,
			Self::Payments => payments::run(ctx).await,
		}
	}
}

pub fn catalog() -> Vec<&'static Check> {
	Section::ALL.iter().flat_map(|section| section.checks().iter().copied()).collect()
}

pub async fn run_by_section(ctx: &Context, mut on_section: impl FnMut(Section, Vec<Outcome>)) {
	let mut pending: FuturesUnordered<_> = Section::ALL
		.iter()
		.enumerate()
		.map(|(index, section)| async move { (index, section.run(ctx).await) })
		.collect();

	let mut ready: Vec<Option<Vec<Outcome>>> = vec![None; Section::ALL.len()];
	let mut next = 0;
	while let Some((index, outcomes)) = pending.next().await {
		ready[index] = Some(outcomes);
		while let Some(outcomes) = ready.get_mut(next).and_then(Option::take) {
			on_section(Section::ALL[next], outcomes);
			next += 1;
		}
	}
}

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let mut all = Vec::new();
	run_by_section(ctx, |_, outcomes| all.extend(outcomes)).await;
	all
}

fn describe(reply: &Result<Reply, WalletError>) -> String {
	match reply {
		Ok(reply) => match reply.error_code() {
			Some(code) => format!("replied with error {code}"),
			None => "replied successfully".to_owned(),
		},
		Err(error) => error.to_string(),
	}
}
