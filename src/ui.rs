use std::fmt::Write as _;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use console::{Style, style};
use indicatif::{ProgressBar, ProgressStyle};
use nwc_check::check::{Level, Outcome, Severity, Status, Summary};
use nwc_check::checks::{self, Section};

pub struct Ui {
	bar: ProgressBar,
	title_width: usize,
}

impl Ui {
	pub fn new(color: bool, live: bool) -> Self {
		console::set_colors_enabled(color);
		console::set_colors_enabled_stderr(color);
		let bar = if live { ProgressBar::new_spinner() } else { ProgressBar::hidden() };
		bar.set_style(
			ProgressStyle::with_template("{spinner:.cyan} {msg} {elapsed:.dim}")
				.unwrap_or_else(|_| ProgressStyle::default_spinner()),
		);
		bar.enable_steady_tick(Duration::from_millis(80));
		let title_width =
			checks::catalog().iter().map(|check| check.title.chars().count()).max().unwrap_or(0);
		Self { bar, title_width }
	}

	pub fn header(&self, wallet: &str, relays: &[String]) {
		self.print(format!(
			"{} {}\n\n  {}  {}\n  {}  {}\n",
			style("nwc-check").bold(),
			style(env!("CARGO_PKG_VERSION")).dim(),
			style("Wallet").dim(),
			short_key(wallet),
			style("Relays").dim(),
			relays.join(", "),
		));
	}

	pub fn busy(&self, message: impl Into<String>) {
		self.bar.set_message(message.into());
	}

	pub fn step(&self, ok: bool, message: impl AsRef<str>) {
		let mark = if ok { style("✔").green() } else { style("!").yellow() };
		self.print(format!("{mark} {}", message.as_ref()));
	}

	pub fn section(&self, section: Section, outcomes: &[Outcome]) {
		if outcomes.is_empty() {
			return;
		}
		let width = self.title_width;
		let mut out = format!("\n{}", style(section.title()).bold().underlined());
		for outcome in outcomes {
			let detail = if outcome.status == Status::Pass {
				style(outcome.detail.as_str()).dim()
			} else {
				status_style(outcome.status).apply_to(outcome.detail.as_str())
			};
			out.push_str(&format!(
				"\n  {} {:<8} {:<width$}  {}",
				mark(outcome.status),
				style(outcome.id).dim(),
				outcome.title,
				detail,
			));
		}
		self.print(out);
	}

	pub fn problems(&self, outcomes: &[Outcome]) {
		let mut problems: Vec<&Outcome> = outcomes.iter().filter(|o| o.is_problem()).collect();
		if problems.is_empty() {
			return;
		}
		problems.sort_by_key(|o| (o.severity, o.status != Status::Fail));

		let mut out = format!("\n{}", style(format!("Problems found ({})", problems.len())).bold());
		for problem in problems {
			out.push_str(&format!(
				"\n\n  {} {} {}  {} {}\n    {}  {}\n    {}  {}",
				mark(problem.status),
				style(problem.id).bold(),
				problem.title,
				severity_badge(problem.severity),
				style(format!("· {}", problem.level.as_str())).dim(),
				style("Found").dim(),
				problem.detail,
				style("Why  ").dim(),
				problem.why,
			));
			if let Some(url) = problem.spec.url() {
				out.push_str(&format!("\n    {}  {}", style("Spec ").dim(), url));
			}
		}
		self.print(out);
	}

	pub fn summary(&self, summary: &Summary, elapsed: Duration, strict: bool) {
		let verdict = if summary.failed > 0 || (strict && summary.warnings > 0) {
			style("✖ Wallet has problems").red().bold()
		} else if summary.warnings > 0 {
			style("⚠ Passed with warnings").yellow().bold()
		} else {
			style("✔ All checks passed").green().bold()
		};
		self.print(format!(
			"\n{verdict}  {} passed · {} {} · {} failed · {} skipped  {}",
			style(summary.passed).green(),
			style(summary.warnings).yellow(),
			if summary.warnings == 1 { "warning" } else { "warnings" },
			style(summary.failed).red(),
			style(summary.skipped).dim(),
			style(format!("({})", human_duration(elapsed))).dim(),
		));
		let by_severity: Vec<String> = Severity::ALL
			.into_iter()
			.filter(|&severity| summary.problems.get(severity) > 0)
			.map(|severity| {
				let count = summary.problems.get(severity);
				severity_style(severity)
					.apply_to(format!("{count} {}", severity.as_str().to_lowercase()))
					.to_string()
			})
			.collect();
		if !by_severity.is_empty() {
			self.print(format!("  {}", by_severity.join(" · ")));
		}
	}

	pub fn saved(&self, label: &str, path: &Path) {
		self.print(format!("\n{} {label}: {}", style("→").cyan(), file_url(path)));
	}

	pub fn finish(&self) {
		self.bar.finish_and_clear();
	}

	fn print(&self, text: String) {
		self.bar.suspend(|| write_stdout(&format!("{text}\n")));
	}
}

pub fn catalog() -> String {
	let width =
		checks::catalog().iter().map(|check| check.title.chars().count()).max().unwrap_or(0);
	let mut out = String::new();
	for section in Section::ALL {
		out.push_str(&format!("{}\n", style(section.title()).bold()));
		for check in section.checks() {
			let spec = check.spec.id().unwrap_or("safety, no spec");
			out.push_str(&format!(
				"  {:<8} {}  {}  {:<width$}  {}\n",
				check.id,
				severity_style(check.severity).apply_to(format!("{:<8}", check.severity.as_str())),
				level_badge(check.level),
				check.title,
				style(spec).dim(),
			));
		}
		out.push('\n');
	}
	out
}

fn mark(status: Status) -> console::StyledObject<&'static str> {
	let symbol = match status {
		Status::Pass => "✔",
		Status::Warn => "⚠",
		Status::Fail => "✖",
		Status::Skip => "○",
	};
	status_style(status).apply_to(symbol)
}

fn status_style(status: Status) -> Style {
	match status {
		Status::Pass => Style::new().green(),
		Status::Warn => Style::new().yellow(),
		Status::Fail => Style::new().red(),
		Status::Skip => Style::new().dim(),
	}
}

fn severity_badge(severity: Severity) -> console::StyledObject<&'static str> {
	severity_style(severity).apply_to(severity.as_str())
}

fn severity_style(severity: Severity) -> Style {
	match severity {
		Severity::Critical => Style::new().red().bold(),
		Severity::High => Style::new().red(),
		Severity::Medium => Style::new().yellow(),
		Severity::Low => Style::new().dim(),
	}
}

fn level_badge(level: Level) -> console::StyledObject<String> {
	let text = format!("{:<6}", level.as_str());
	match level {
		Level::Must => style(text).cyan(),
		Level::Should => style(text).blue(),
		Level::Safety => style(text).magenta(),
	}
}

fn short_key(key: &str) -> String {
	if key.len() > 16 {
		format!("{}…{}", &key[..8], &key[key.len() - 8..])
	} else {
		key.to_owned()
	}
}

fn human_duration(elapsed: Duration) -> String {
	if elapsed.as_secs() == 0 {
		format!("{} ms", elapsed.as_millis())
	} else {
		format!("{:.1}s", elapsed.as_secs_f64())
	}
}

pub fn write_stdout(text: &str) {
	let _ = std::io::stdout().write_all(text.as_bytes());
}

/// A file:// URL for the path, which most terminals make clickable.
fn file_url(path: &Path) -> String {
	let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
	let mut url = String::from("file://");
	for byte in absolute.to_string_lossy().bytes() {
		match byte {
			b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
				url.push(char::from(byte));
			}
			_ => {
				let _ = write!(url, "%{byte:02X}");
			}
		}
	}
	url
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn file_urls_are_absolute_and_encoded() {
		assert_eq!(file_url(Path::new("/tmp/my report.html")), "file:///tmp/my%20report.html");
		assert!(file_url(Path::new("report.html")).starts_with("file:///"));
	}
}
