mod json;
mod text;

use serde::Serialize;

use crate::check::{Outcome, Summary};

pub use json::render as json;
pub use text::{catalog, render as text};

#[derive(Debug, Serialize)]
pub struct Report {
	pub tool: &'static str,
	pub version: &'static str,
	pub wallet: String,
	pub relays: Vec<String>,
	pub summary: Summary,
	pub results: Vec<Outcome>,
}

impl Report {
	pub fn new(wallet: String, relays: Vec<String>, results: Vec<Outcome>) -> Self {
		Self {
			tool: env!("CARGO_PKG_NAME"),
			version: env!("CARGO_PKG_VERSION"),
			wallet,
			relays,
			summary: Summary::of(&results),
			results,
		}
	}
}
