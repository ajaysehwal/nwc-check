mod discovery;
mod encryption;
mod errors;
mod response;
mod safety;

use nostr_sdk::prelude::Event;

use crate::check::{Check, Outcome};
use crate::wallet::{Encryption, Reply, Request, Scheme, Wallet, WalletError};

pub const UNKNOWN_METHOD: &str = "nwc_check_unknown_method";

pub struct Context {
	pub wallet: Wallet,
	pub info: Option<Event>,
	pub methods: Vec<String>,
	pub schemes: Vec<String>,
	pub expected_network: Option<String>,
	pub get_info: Result<Reply, WalletError>,
	pub unknown_method: Result<Reply, WalletError>,
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

		Ok(Self { wallet, info, methods, schemes, expected_network, get_info, unknown_method })
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

pub fn catalog() -> Vec<&'static Check> {
	[discovery::CHECKS, encryption::CHECKS, response::CHECKS, errors::CHECKS, safety::CHECKS]
		.into_iter()
		.flatten()
		.copied()
		.collect()
}

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let (discovery, encryption, response, errors, safety) = tokio::join!(
		discovery::run(ctx),
		encryption::run(ctx),
		response::run(ctx),
		errors::run(ctx),
		safety::run(ctx),
	);
	[discovery, encryption, response, errors, safety].into_iter().flatten().collect()
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
