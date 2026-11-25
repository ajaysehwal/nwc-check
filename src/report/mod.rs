mod html;
mod junit;

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

	pub fn to_junit(&self) -> String {
		junit::render(self)
	}

	/// Whether the run counts as failed: any failure, or any warning in strict mode.
	pub fn failed(&self) -> bool {
		self.summary.failed > 0 || (self.strict && self.summary.warnings > 0)
	}
}

/// Escapes text for HTML and XML, dropping control characters XML doesn't allow.
fn escape(text: &str) -> String {
	let mut escaped = String::with_capacity(text.len());
	for character in text.chars() {
		match character {
			'&' => escaped.push_str("&amp;"),
			'<' => escaped.push_str("&lt;"),
			'>' => escaped.push_str("&gt;"),
			'"' => escaped.push_str("&quot;"),
			'\'' => escaped.push_str("&#39;"),
			'\t' | '\n' | '\r' => escaped.push(character),
			control if control.is_control() => {}
			other => escaped.push(other),
		}
	}
	escaped
}

/// Splits Unix seconds into a UTC date and time.
fn civil(secs: u64) -> (i64, i64, i64, u64, u64, u64) {
	let days = (secs / 86_400) as i64;
	// Civil-from-days, from Howard Hinnant's date algorithms.
	let z = days + 719_468;
	let era = z.div_euclid(146_097);
	let doe = z.rem_euclid(146_097);
	let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let day = doy - (153 * mp + 2) / 5 + 1;
	let month = if mp < 10 { mp + 3 } else { mp - 9 };
	let year = yoe + era * 400 + i64::from(month <= 2);
	(year, month, day, (secs % 86_400) / 3600, (secs % 3600) / 60, secs % 60)
}

/// For people, e.g. "2026-11-22 14:05 UTC".
fn utc(secs: u64) -> String {
	let (year, month, day, hour, minute, _) = civil(secs);
	format!("{year}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

/// For machines, e.g. "2026-11-22T14:05:09".
fn iso8601(secs: u64) -> String {
	let (year, month, day, hour, minute, second) = civil(secs);
	format!("{year}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}")
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::check::{Check, Level, Severity, Spec};

	const PINNED: Check =
		Check::new("T-01", Level::Must, Severity::High, Spec::Nip47, "Pinned", "Why.");
	const UNSPECIFIED: Check =
		Check::new("T-02", Level::Safety, Severity::Medium, Spec::Safety, "Unpinned", "Why.");

	fn report() -> Report {
		let mut results = vec![PINNED.pass("ok"), UNSPECIFIED.fail("bad")];
		results.iter_mut().for_each(|outcome| outcome.section = "Tests");
		Report::new("wallet".to_owned(), vec!["wss://relay".to_owned()], 5, false, results)
	}

	#[test]
	fn formats_dates() {
		assert_eq!(utc(0), "1970-01-01 00:00 UTC");
		assert_eq!(utc(1_795_000_000), "2026-11-18 11:06 UTC");
		assert_eq!(iso8601(1_795_000_000), "2026-11-18T11:06:40");
	}

	#[test]
	fn escaping_drops_control_characters() {
		assert_eq!(escape("a<b & \"c\"\u{0}\u{1b}"), "a&lt;b &amp; &quot;c&quot;");
	}

	#[test]
	fn summary_counts_problems_by_severity() {
		let json: serde_json::Value = serde_json::from_str(&report().to_json()).unwrap();
		assert_eq!(json["results"][0]["severity"], "high");
		let problems = &json["summary"]["problems"];
		assert_eq!(problems["high"], 0, "a pass isn't a problem");
		assert_eq!(problems["medium"], 1);
		assert_eq!(problems["critical"], 0);
	}

	#[test]
	fn json_cites_the_spec_and_section() {
		let json: serde_json::Value = serde_json::from_str(&report().to_json()).unwrap();
		assert_eq!(json["results"][0]["spec"], "nip47@c53877571f96");
		assert_eq!(json["results"][0]["section"], "Tests");
		assert!(json["results"][1]["spec"].is_null());
	}
}
