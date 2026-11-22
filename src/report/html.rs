use std::fmt::Write as _;

use super::{Report, escape, utc};
use crate::check::{Level, Outcome, Status};
use crate::evidence::Exchange;

const STYLE: &str = r#"
:root {
	--bg: #ffffff; --surface: #f6f7f9; --border: #e2e5ea; --text: #1b1f24; --muted: #5f6b7a;
	--pass: #1a7f37; --warn: #9a6700; --fail: #cf222e; --skip: #6e7781; --link: #0969da;
}
@media (prefers-color-scheme: dark) {
	:root {
		--bg: #0d1117; --surface: #161b22; --border: #30363d; --text: #e6edf3; --muted: #8b949e;
		--pass: #3fb950; --warn: #d29922; --fail: #f85149; --skip: #8b949e; --link: #58a6ff;
	}
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--text);
	font: 15px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
main { max-width: 960px; margin: 0 auto; padding: 32px 16px 64px; }
h1 { font-size: 22px; margin: 0 0 4px; }
h2 { font-size: 17px; margin: 32px 0 12px; }
a { color: var(--link); }
code, .id { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 13px; }
code, dd, td { overflow-wrap: anywhere; }
.meta { color: var(--muted); margin: 0; word-break: break-all; }
.verdict { margin: 24px 0 0; padding: 16px; border-radius: 8px; border: 1px solid var(--border);
	background: var(--surface); font-weight: 600; }
.verdict.pass { border-left: 4px solid var(--pass); }
.verdict.warn { border-left: 4px solid var(--warn); }
.verdict.fail { border-left: 4px solid var(--fail); }
.verdict span { font-weight: 400; color: var(--muted); margin-left: 8px; }
.problem { border: 1px solid var(--border); border-radius: 8px; padding: 12px 16px; margin: 0 0 12px; }
.problem dl { display: grid; grid-template-columns: max-content minmax(0, 1fr); gap: 4px 12px; margin: 8px 0 0; }
.problem dt { color: var(--muted); }
.problem dd { margin: 0; }
table { width: 100%; border-collapse: collapse; }
td { padding: 8px 6px; border-top: 1px solid var(--border); vertical-align: top; }
td.status { width: 24px; }
.level { font-size: 12px; color: var(--muted); white-space: nowrap; }
.detail { color: var(--muted); }
.pass .mark { color: var(--pass); } .warn .mark { color: var(--warn); }
.fail .mark { color: var(--fail); } .skip .mark { color: var(--skip); }
details { margin-top: 4px; }
summary { cursor: pointer; color: var(--muted); font-size: 13px; }
details ul { margin: 4px 0 0; padding-left: 18px; font-size: 13px; }
@media (max-width: 640px) { td.level-cell, td.time { display: none; } }
"#;

pub(super) fn render(report: &Report) -> String {
	let mut out = String::new();
	let summary = &report.summary;
	let (verdict, class) = if report.failed() {
		("Wallet has problems", "fail")
	} else if summary.warnings > 0 {
		("Passed with warnings", "warn")
	} else {
		("All checks passed", "pass")
	};

	let _ = write!(
		out,
		"<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
		<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
		<title>nwc-check report</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<main>\n\
		<h1>nwc-check report</h1>\n\
		<p class=\"meta\">Wallet <code>{}</code></p>\n\
		<p class=\"meta\">Relays {}</p>\n\
		<p class=\"meta\">{} · nwc-check {} · {} ms</p>\n\
		<p class=\"verdict {class}\">{verdict}<span>{} passed · {} warnings · {} failed · {} skipped</span></p>\n",
		escape(&report.wallet),
		escape(&report.relays.join(", ")),
		utc(report.generated_at),
		escape(report.version),
		report.duration_ms,
		summary.passed,
		summary.warnings,
		summary.failed,
		summary.skipped,
	);

	let mut problems: Vec<&Outcome> = report.results.iter().filter(|o| o.is_problem()).collect();
	problems.sort_by_key(|o| (o.status != Status::Fail, o.level != Level::Safety));
	if !problems.is_empty() {
		let _ = writeln!(out, "<h2>Problems found ({})</h2>", problems.len());
		for problem in problems {
			problem_card(&mut out, problem);
		}
	}

	let mut sections: Vec<&str> = Vec::new();
	for outcome in &report.results {
		if !sections.contains(&outcome.section) {
			sections.push(outcome.section);
		}
	}
	for section in sections {
		let _ = writeln!(out, "<h2>{}</h2>\n<table>", escape(section));
		for outcome in report.results.iter().filter(|o| o.section == section) {
			result_row(&mut out, outcome);
		}
		out.push_str("</table>\n");
	}
	out.push_str("</main>\n</body>\n</html>\n");
	out
}

fn problem_card(out: &mut String, problem: &Outcome) {
	let _ = write!(
		out,
		"<div class=\"problem {}\">\n<strong><span class=\"mark\">{}</span> <span class=\"id\">{}</span> {}</strong> \
		<span class=\"level\">{}</span>\n<dl>\n<dt>Found</dt><dd>{}</dd>\n<dt>Why</dt><dd>{}</dd>\n",
		class(problem.status),
		mark(problem.status),
		escape(problem.id),
		escape(problem.title),
		problem.level.as_str(),
		escape(&problem.detail),
		escape(problem.why),
	);
	if let (Some(id), Some(url)) = (problem.spec.id(), problem.spec.url()) {
		let _ =
			writeln!(out, "<dt>Spec</dt><dd><a href=\"{}\">{}</a></dd>", escape(url), escape(id));
	}
	if !problem.evidence.is_empty() {
		out.push_str("<dt>Requests</dt><dd>");
		evidence_list(out, &problem.evidence);
		out.push_str("</dd>\n");
	}
	out.push_str("</dl>\n</div>\n");
}

fn result_row(out: &mut String, outcome: &Outcome) {
	let spec = match (outcome.spec.id(), outcome.spec.url()) {
		(Some(id), Some(url)) => format!("<a href=\"{}\">{}</a>", escape(url), escape(id)),
		_ => "no spec".to_owned(),
	};
	let time = outcome.duration_ms.map(|ms| format!("{ms} ms")).unwrap_or_default();
	let _ = write!(
		out,
		"<tr class=\"{}\"><td class=\"status mark\">{}</td><td><span class=\"id\">{}</span> {}\
		<div class=\"detail\">{}</div>",
		class(outcome.status),
		mark(outcome.status),
		escape(outcome.id),
		escape(outcome.title),
		escape(&outcome.detail),
	);
	if !outcome.evidence.is_empty() {
		let count = outcome.evidence.len();
		let noun = if count == 1 { "request" } else { "requests" };
		let _ = write!(out, "<details><summary>{count} {noun}</summary>");
		evidence_list(out, &outcome.evidence);
		out.push_str("</details>");
	}
	let _ = writeln!(
		out,
		"</td><td class=\"level-cell level\">{} · {spec}</td><td class=\"time level\">{time}</td></tr>",
		outcome.level.as_str(),
	);
}

fn evidence_list(out: &mut String, exchanges: &[Exchange]) {
	out.push_str("<ul>");
	for exchange in exchanges {
		let reply = match &exchange.reply {
			Some(reply) => format!("reply <code>{}</code>", escape(reply)),
			None => "no reply".to_owned(),
		};
		let _ = write!(
			out,
			"<li><code>{}</code> request <code>{}</code>, {reply}, {} ms</li>",
			escape(&exchange.method),
			escape(&exchange.request),
			exchange.ms,
		);
	}
	out.push_str("</ul>");
}

fn class(status: Status) -> &'static str {
	match status {
		Status::Pass => "pass",
		Status::Warn => "warn",
		Status::Fail => "fail",
		Status::Skip => "skip",
	}
}

fn mark(status: Status) -> &'static str {
	match status {
		Status::Pass => "✔",
		Status::Warn => "⚠",
		Status::Fail => "✖",
		Status::Skip => "○",
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::check::{Check, Spec};

	const CHECK: Check = Check::new("T-01", Level::Must, Spec::Nip47, "A check", "Why.");

	#[test]
	fn wallet_text_is_escaped() {
		let mut outcome = CHECK.fail("<script>alert(1)</script>");
		outcome.section = "Tests";
		let report = Report::new("w".to_owned(), vec![], 1, false, vec![outcome]);
		let html = render(&report);
		assert!(!html.contains("<script>"));
		assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
		assert!(html.contains("Wallet has problems"));
		assert!(html.contains("https://github.com/nostr-protocol/nips/blob/c53877571f96/47.md"));
	}
}
