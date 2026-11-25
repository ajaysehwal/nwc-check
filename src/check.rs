use serde::{Serialize, Serializer};

use crate::evidence::{Evidence, Exchange};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
	Must,
	Should,
	Safety,
}

impl Level {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Must => "MUST",
			Self::Should => "SHOULD",
			Self::Safety => "SAFETY",
		}
	}
}

/// How much harm a problem can do to the user, which decides what to fix first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
	/// Funds can be lost beyond what the user allowed.
	Critical,
	/// Money can be wrong, lost or misreported.
	High,
	/// Apps can break or misread the wallet.
	Medium,
	/// A detail that rarely affects users.
	Low,
}

impl Severity {
	pub const ALL: [Severity; 4] = [Self::Critical, Self::High, Self::Medium, Self::Low];

	pub fn as_str(self) -> &'static str {
		match self {
			Self::Critical => "CRITICAL",
			Self::High => "HIGH",
			Self::Medium => "MEDIUM",
			Self::Low => "LOW",
		}
	}
}

/// The spec a check comes from, pinned to the commit the check was written against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spec {
	Nip47,
	Nip40,
	Nwc13,
	/// A safety rule no spec requires.
	Safety,
}

impl Spec {
	pub fn id(self) -> Option<&'static str> {
		match self {
			Self::Nip47 => Some("nip47@c53877571f96"),
			Self::Nip40 => Some("nip40@f5a15ea27e06"),
			Self::Nwc13 => Some("nwc13@36ab132bbce1"),
			Self::Safety => None,
		}
	}

	pub fn url(self) -> Option<&'static str> {
		match self {
			Self::Nip47 => Some("https://github.com/nostr-protocol/nips/blob/c53877571f96/47.md"),
			Self::Nip40 => Some("https://github.com/nostr-protocol/nips/blob/f5a15ea27e06/40.md"),
			Self::Nwc13 => {
				Some("https://github.com/nostr-wallet-connect/nwc/blob/36ab132bbce1/13.md")
			}
			Self::Safety => None,
		}
	}
}

impl Serialize for Spec {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		self.id().serialize(serializer)
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
	Pass,
	Warn,
	Fail,
	Skip,
}

#[derive(Debug)]
pub struct Check {
	pub id: &'static str,
	pub level: Level,
	pub severity: Severity,
	pub spec: Spec,
	pub title: &'static str,
	pub why: &'static str,
}

impl Check {
	pub const fn new(
		id: &'static str,
		level: Level,
		severity: Severity,
		spec: Spec,
		title: &'static str,
		why: &'static str,
	) -> Self {
		Self { id, level, severity, spec, title, why }
	}

	pub fn pass(&self, detail: impl Into<String>) -> Outcome {
		self.outcome(Status::Pass, detail)
	}

	pub fn fail(&self, detail: impl Into<String>) -> Outcome {
		let status = if self.level == Level::Should { Status::Warn } else { Status::Fail };
		self.outcome(status, detail)
	}

	pub fn skip(&self, detail: impl Into<String>) -> Outcome {
		self.outcome(Status::Skip, detail)
	}

	pub fn verdict(&self, passed: bool, detail: impl Into<String>) -> Outcome {
		if passed { self.pass(detail) } else { self.fail(detail) }
	}

	fn outcome(&self, status: Status, detail: impl Into<String>) -> Outcome {
		Outcome {
			id: self.id,
			section: "",
			level: self.level,
			severity: self.severity,
			spec: self.spec,
			title: self.title,
			duration_ms: None,
			evidence: Vec::new(),
			why: self.why,
			status,
			detail: detail.into(),
		}
	}
}

#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
	pub id: &'static str,
	pub section: &'static str,
	pub level: Level,
	pub severity: Severity,
	pub spec: Spec,
	pub title: &'static str,
	pub why: &'static str,
	pub duration_ms: Option<u64>,
	pub evidence: Vec<Exchange>,
	pub status: Status,
	pub detail: String,
}

impl Outcome {
	/// Attaches what the code behind this outcome sent, and how long it took.
	pub fn with(mut self, evidence: &Evidence) -> Self {
		self.evidence.extend(evidence.exchanges.iter().cloned());
		self.duration_ms = Some(evidence.took.as_millis().try_into().unwrap_or(u64::MAX));
		self
	}

	/// Cites exchanges made elsewhere, such as the shared get_info reply.
	pub fn citing(mut self, exchanges: &[Exchange]) -> Self {
		self.evidence.extend(exchanges.iter().cloned());
		self
	}

	pub fn is_problem(&self) -> bool {
		matches!(self.status, Status::Fail | Status::Warn)
	}
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Summary {
	pub passed: usize,
	pub warnings: usize,
	pub failed: usize,
	pub skipped: usize,
	/// Failures and warnings, by severity.
	pub problems: BySeverity,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct BySeverity {
	pub critical: usize,
	pub high: usize,
	pub medium: usize,
	pub low: usize,
}

impl BySeverity {
	pub fn get(&self, severity: Severity) -> usize {
		match severity {
			Severity::Critical => self.critical,
			Severity::High => self.high,
			Severity::Medium => self.medium,
			Severity::Low => self.low,
		}
	}

	fn add(&mut self, severity: Severity) {
		match severity {
			Severity::Critical => self.critical += 1,
			Severity::High => self.high += 1,
			Severity::Medium => self.medium += 1,
			Severity::Low => self.low += 1,
		}
	}
}

impl Summary {
	pub fn of(outcomes: &[Outcome]) -> Self {
		let mut summary = Self::default();
		for outcome in outcomes {
			match outcome.status {
				Status::Pass => summary.passed += 1,
				Status::Warn => summary.warnings += 1,
				Status::Fail => summary.failed += 1,
				Status::Skip => summary.skipped += 1,
			}
			if outcome.is_problem() {
				summary.problems.add(outcome.severity);
			}
		}
		summary
	}
}
