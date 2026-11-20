mod lnd;

use std::fmt;

use anyhow::Result;
use bitcoin_hashes::sha256;
use futures::future::BoxFuture;

pub use lnd::Lnd;

pub struct NodeInfo {
	pub pubkey: String,
	pub network: String,
}

pub struct Invoice {
	pub bolt11: String,
	pub payment_hash: [u8; 32],
}

#[derive(Debug)]
pub enum PayError {
	/// The counterparty couldn't attempt the payment, so the wallet isn't at fault.
	Unable(String),
	/// The payment was attempted and failed.
	Failed(String),
}

impl fmt::Display for PayError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Unable(reason) | Self::Failed(reason) => f.write_str(reason),
		}
	}
}

impl std::error::Error for PayError {}

pub trait Counterparty: Send + Sync {
	fn info(&self) -> BoxFuture<'_, Result<NodeInfo>>;

	fn create_invoice<'a>(
		&'a self,
		amount_msat: u64,
		memo: &'a str,
	) -> BoxFuture<'a, Result<Invoice>>;

	fn is_settled<'a>(&'a self, payment_hash: &'a [u8; 32]) -> BoxFuture<'a, Result<bool>>;

	fn pay<'a>(&'a self, bolt11: &'a str) -> BoxFuture<'a, Result<(), PayError>>;
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
	sha256::Hash::hash(data).to_byte_array()
}

pub fn to_hex(bytes: &[u8]) -> String {
	bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn from_hex(hex: &str) -> Option<Vec<u8>> {
	if !hex.len().is_multiple_of(2) {
		return None;
	}
	(0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect()
}
