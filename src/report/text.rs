use std::fmt::Write;

use super::Report;
use crate::check::{Check, Status};

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";

fn status_style(status: Status) -> (&'static str, &'static str) {
	match status {
		Status::Pass => ("PASS", "\x1b[32m"),
		Status::Warn => ("WARN", "\x1b[33m"),
		Status::Fail => ("FAIL", "\x1b[31m"),
		Status::Skip => ("SKIP", DIM),
	}
}

struct Painter {
	color: bool,
}

impl Painter {
	fn paint(&self, style: &str, text: &str) -> String {
		if self.color { format!("{style}{text}{RESET}") } else { text.to_owned() }
	}
}

pub fn render(report: &Report, color: bool) -> String {
	let painter = Painter { color };
	let mut out = String::new();

	let _ = writeln!(out, "{}", painter.paint(BOLD, &format!("nwc-check {}", report.version)));
	let _ = writeln!(out, "wallet  {}", report.wallet);
	let _ = writeln!(out, "relays  {}\n", report.relays.join(", "));

	let title_width = report.results.iter().map(|r| r.title.len()).max().unwrap_or(0);
	for result in &report.results {
		let (label, style) = status_style(result.status);
		let _ = writeln!(
			out,
			"  {}  {:<8} {:<6}  {:<title_width$}  {}",
			painter.paint(style, label),
			result.id,
			result.level.as_str(),
			result.title,
			painter.paint(DIM, &result.detail),
		);
	}

	let summary = &report.summary;
	let _ = writeln!(
		out,
		"\n{} passed, {} warnings, {} failed, {} skipped",
		painter.paint("\x1b[32m", &summary.passed.to_string()),
		painter.paint("\x1b[33m", &summary.warnings.to_string()),
		painter.paint("\x1b[31m", &summary.failed.to_string()),
		summary.skipped,
	);
	out
}

pub fn catalog(checks: &[&Check]) -> String {
	let mut out = String::new();
	for check in checks {
		let _ = writeln!(out, "{:<8} {:<6}  {}", check.id, check.level.as_str(), check.title);
	}
	out
}
