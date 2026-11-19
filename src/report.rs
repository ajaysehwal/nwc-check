use serde::Serialize;

use crate::check::{Outcome, Summary};

#[derive(Debug, Serialize)]
pub struct Report {
	pub tool: &'static str,
	pub version: &'static str,
	pub wallet: String,
	pub relays: Vec<String>,
	pub duration_ms: u128,
	pub summary: Summary,
	pub results: Vec<Outcome>,
}

impl Report {
	pub fn new(
		wallet: String,
		relays: Vec<String>,
		duration_ms: u128,
		results: Vec<Outcome>,
	) -> Self {
		Self {
			tool: env!("CARGO_PKG_NAME"),
			version: env!("CARGO_PKG_VERSION"),
			wallet,
			relays,
			duration_ms,
			summary: Summary::of(&results),
			results,
		}
	}

	pub fn to_json(&self) -> String {
		serde_json::to_string_pretty(self).unwrap_or_default()
	}
}
