mod html;

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::check::{Outcome, Summary};

#[derive(Debug, Serialize)]
pub struct Report {
	pub tool: &'static str,
	pub version: &'static str,
	pub wallet: String,
	pub relays: Vec<String>,
	pub generated_at: u64,
	pub duration_ms: u128,
	pub strict: bool,
	pub summary: Summary,
	pub results: Vec<Outcome>,
}

impl Report {
	pub fn new(
		wallet: String,
		relays: Vec<String>,
		duration_ms: u128,
		strict: bool,
		results: Vec<Outcome>,
	) -> Self {
		let generated_at =
			SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
		Self {
			tool: env!("CARGO_PKG_NAME"),
			version: env!("CARGO_PKG_VERSION"),
			wallet,
			relays,
			generated_at,
			duration_ms,
			strict,
			summary: Summary::of(&results),
			results,
		}
	}

	pub fn to_json(&self) -> String {
		serde_json::to_string_pretty(self).unwrap_or_default()
	}

	pub fn to_html(&self) -> String {
		html::render(self)
	}

	/// Whether the run counts as failed: any failure, or any warning in strict mode.
	pub fn failed(&self) -> bool {
		self.summary.failed > 0 || (self.strict && self.summary.warnings > 0)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::check::{Check, Level, Spec};

	const PINNED: Check = Check::new("T-01", Level::Must, Spec::Nip47, "Pinned", "Why.");
	const UNSPECIFIED: Check = Check::new("T-02", Level::Safety, Spec::Safety, "Unpinned", "Why.");

	fn report() -> Report {
		let mut results = vec![PINNED.pass("ok"), UNSPECIFIED.fail("bad")];
		results.iter_mut().for_each(|outcome| outcome.section = "Tests");
		Report::new("wallet".to_owned(), vec!["wss://relay".to_owned()], 5, false, results)
	}

	#[test]
	fn json_cites_the_spec_and_section() {
		let json: serde_json::Value = serde_json::from_str(&report().to_json()).unwrap();
		assert_eq!(json["results"][0]["spec"], "nip47@c53877571f96");
		assert_eq!(json["results"][0]["section"], "Tests");
		assert!(json["results"][1]["spec"].is_null());
	}
}
