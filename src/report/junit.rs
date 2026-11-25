use std::fmt::Write as _;

use super::{Report, escape, iso8601};
use crate::check::{Outcome, Status};

pub(super) fn render(report: &Report) -> String {
	let mut sections: Vec<&str> = Vec::new();
	for outcome in &report.results {
		if !sections.contains(&outcome.section) {
			sections.push(outcome.section);
		}
	}

	let failures = report.results.iter().filter(|o| fails(report, o)).count();
	let skipped = report.results.iter().filter(|o| o.status == Status::Skip).count();
	let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
	let _ = writeln!(
		out,
		"<testsuites name=\"nwc-check\" tests=\"{}\" failures=\"{failures}\" skipped=\"{skipped}\" \
		time=\"{}\" timestamp=\"{}\">",
		report.results.len(),
		seconds(report.duration_ms.try_into().unwrap_or(u64::MAX)),
		iso8601(report.generated_at),
	);
	for section in sections {
		let outcomes: Vec<&Outcome> =
			report.results.iter().filter(|o| o.section == section).collect();
		let _ = writeln!(
			out,
			"  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" skipped=\"{}\" time=\"{}\">",
			escape(section),
			outcomes.len(),
			outcomes.iter().filter(|o| fails(report, o)).count(),
			outcomes.iter().filter(|o| o.status == Status::Skip).count(),
			seconds(outcomes.iter().filter_map(|o| o.duration_ms).sum()),
		);
		for outcome in outcomes {
			testcase(&mut out, report, outcome);
		}
		out.push_str("  </testsuite>\n");
	}
	out.push_str("</testsuites>\n");
	out
}

fn testcase(out: &mut String, report: &Report, outcome: &Outcome) {
	let _ = writeln!(
		out,
		"    <testcase classname=\"nwc-check.{}\" name=\"{} {}\" time=\"{}\">",
		escape(outcome.section),
		escape(outcome.id),
		escape(outcome.title),
		seconds(outcome.duration_ms.unwrap_or(0)),
	);
	let _ = writeln!(
		out,
		"      <properties>\n        <property name=\"severity\" value=\"{}\"/>\n        \
		<property name=\"level\" value=\"{}\"/>\n        \
		<property name=\"spec\" value=\"{}\"/>\n      </properties>",
		outcome.severity.as_str(),
		outcome.level.as_str(),
		escape(outcome.spec.id().unwrap_or("none")),
	);
	if fails(report, outcome) {
		let _ = writeln!(
			out,
			"      <failure type=\"{}\" message=\"{}\">{}</failure>",
			outcome.severity.as_str(),
			escape(&outcome.detail),
			escape(&explanation(outcome)),
		);
	} else if outcome.status == Status::Skip {
		let _ = writeln!(out, "      <skipped message=\"{}\"/>", escape(&outcome.detail));
	} else if outcome.status == Status::Warn {
		let text = format!("Warning: {}\n{}", outcome.detail, explanation(outcome));
		let _ = writeln!(out, "      <system-out>{}</system-out>", escape(&text));
	}
	out.push_str("    </testcase>\n");
}

/// A warning only fails in strict mode, matching the exit code.
fn fails(report: &Report, outcome: &Outcome) -> bool {
	outcome.status == Status::Fail || (report.strict && outcome.status == Status::Warn)
}

fn explanation(outcome: &Outcome) -> String {
	let mut text = format!("Why: {}", outcome.why);
	if let Some(url) = outcome.spec.url() {
		let _ = write!(text, "\nSpec: {url}");
	}
	for exchange in &outcome.evidence {
		let reply = exchange.reply.as_deref().unwrap_or("none");
		let _ = write!(
			text,
			"\nRequest: {} {} reply {reply} ({} ms)",
			exchange.method, exchange.request, exchange.ms
		);
	}
	text
}

fn seconds(ms: u64) -> String {
	format!("{}.{:03}", ms / 1000, ms % 1000)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::check::{Check, Level, Severity, Spec};

	const MUST: Check = Check::new(
		"T-01",
		Level::Must,
		Severity::High,
		Spec::Nip47,
		"Must & co",
		"Why it matters.",
	);
	const SHOULD: Check =
		Check::new("T-02", Level::Should, Severity::Low, Spec::Safety, "Should", "Why.");

	fn report(strict: bool) -> Report {
		let mut results =
			vec![MUST.fail("got <nothing>"), SHOULD.fail("meh"), MUST.skip("not advertised")];
		results.iter_mut().for_each(|outcome| outcome.section = "Tests");
		Report::new("w".to_owned(), vec![], 1500, strict, results)
	}

	#[test]
	fn failures_carry_detail_why_and_spec() {
		let xml = render(&report(false));
		assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
		assert!(xml.contains("tests=\"3\" failures=\"1\" skipped=\"1\" time=\"1.500\""));
		assert!(xml.contains("name=\"T-01 Must &amp; co\""));
		assert!(xml.contains(
			"<failure type=\"HIGH\" message=\"got &lt;nothing&gt;\">Why: Why it matters."
		));
		assert!(
			xml.contains("Spec: https://github.com/nostr-protocol/nips/blob/c53877571f96/47.md")
		);
		assert!(xml.contains("<skipped message=\"not advertised\"/>"));
		assert!(xml.contains("<system-out>Warning: meh"));
	}

	#[test]
	fn warnings_fail_in_strict_mode() {
		let xml = render(&report(true));
		assert!(xml.contains("failures=\"2\""));
		assert!(xml.contains("<failure type=\"LOW\" message=\"meh\">"));
		assert!(xml.contains("<property name=\"severity\" value=\"LOW\"/>"));
	}
}
