use serde::Serialize;

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
	pub title: &'static str,
}

impl Check {
	pub const fn new(id: &'static str, level: Level, title: &'static str) -> Self {
		Self { id, level, title }
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
		Outcome { id: self.id, level: self.level, title: self.title, status, detail: detail.into() }
	}
}

#[derive(Debug, Serialize)]
pub struct Outcome {
	pub id: &'static str,
	pub level: Level,
	pub title: &'static str,
	pub status: Status,
	pub detail: String,
}

#[derive(Debug, Default, Serialize)]
pub struct Summary {
	pub passed: usize,
	pub warnings: usize,
	pub failed: usize,
	pub skipped: usize,
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
		}
		summary
	}
}
