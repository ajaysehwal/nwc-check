use nostr_sdk::prelude::Timestamp;
use serde_json::{Map, Value};

use super::{Context, describe};
use crate::check::{Check, Level, Outcome, Spec};
use crate::wallet::Request;

const PERIODS: [&str; 5] = ["daily", "weekly", "monthly", "yearly", "never"];

const ADVERTISED: Check = Check::new(
	"EXT13-01",
	Level::Must,
	Spec::Nwc13,
	"get_budget is listed in the info event and get_info",
	"Apps only call get_budget when both list it.",
);
const SHAPE: Check = Check::new(
	"EXT13-02",
	Level::Must,
	Spec::Nwc13,
	"get_budget is empty or has every required field",
	"An empty result means no budget, so a configured budget must never look empty or partial.",
);
const RENEWAL: Check = Check::new(
	"EXT13-03",
	Level::Must,
	Spec::Nwc13,
	"A renewing budget has renews_at, a never budget doesn't",
	"Apps tell users when their allowance resets from renews_at.",
);

pub const CHECKS: &[&Check] = &[&ADVERTISED, &SHAPE, &RENEWAL];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let advertised = advertised(ctx);
	if !ctx.supports("get_budget") {
		let reason = "get_budget not advertised in the info event";
		return vec![advertised, SHAPE.skip(reason), RENEWAL.skip(reason)];
	}
	let reply = ctx.wallet.send(Request::new("get_budget", ctx.encryption())).await;
	let budget = match &reply {
		Ok(reply) if reply.error().is_none() => match reply.result() {
			Some(Value::Object(budget)) => budget.clone(),
			_ => {
				let reason = "reply has no result object";
				return vec![advertised, SHAPE.fail(reason), RENEWAL.skip(reason)];
			}
		},
		_ => {
			let reason = format!("get_budget: {}", describe(&reply));
			return vec![advertised, SHAPE.fail(&reason), RENEWAL.skip(reason)];
		}
	};
	vec![advertised, shape(&budget), renewal(&budget)]
}

fn advertised(ctx: &Context) -> Outcome {
	let in_get_info = ctx
		.get_info
		.as_ref()
		.ok()
		.and_then(|reply| reply.result())
		.and_then(|result| result.get("methods"))
		.and_then(Value::as_array)
		.is_some_and(|methods| methods.iter().any(|method| method == "get_budget"));
	match (ctx.supports("get_budget"), in_get_info) {
		(true, true) => ADVERTISED.pass("listed in both"),
		(false, false) => ADVERTISED.skip("get_budget not advertised"),
		(true, false) => ADVERTISED.fail("in the info event but missing from get_info"),
		(false, true) => ADVERTISED.fail("in get_info but missing from the info event"),
	}
}

fn shape(budget: &Map<String, Value>) -> Outcome {
	if budget.is_empty() {
		return SHAPE.pass("no budget configured");
	}
	let mut problems = Vec::new();
	for field in ["used_budget", "total_budget"] {
		if budget.get(field).and_then(Value::as_u64).is_none() {
			problems.push(format!("{field} is missing or not msats"));
		}
	}
	let period = budget.get("renewal_period").and_then(Value::as_str);
	if !period.is_some_and(|period| PERIODS.contains(&period)) {
		problems.push(format!("renewal_period isn't one of {}", PERIODS.join(", ")));
	}
	if problems.is_empty() {
		let msats = |field| budget.get(field).and_then(Value::as_u64).unwrap_or(0);
		SHAPE.pass(format!(
			"{} of {} sats used, renews {}",
			msats("used_budget") / 1000,
			msats("total_budget") / 1000,
			period.unwrap_or_default()
		))
	} else {
		SHAPE.fail(problems.join("; "))
	}
}

fn renewal(budget: &Map<String, Value>) -> Outcome {
	if budget.is_empty() {
		return RENEWAL.skip("no budget configured");
	}
	let renews_at = budget.get("renews_at");
	match budget.get("renewal_period").and_then(Value::as_str) {
		None => RENEWAL.skip("no renewal_period"),
		Some("never") if renews_at.is_some() => RENEWAL.fail("never renews but has renews_at"),
		Some("never") => RENEWAL.pass("never renews, no renews_at"),
		Some(period) => match renews_at.and_then(Value::as_u64) {
			None => RENEWAL.fail(format!("renews {period} but has no renews_at")),
			Some(at) if at < Timestamp::now().as_secs() => {
				RENEWAL.fail(format!("renews_at {at} is in the past"))
			}
			Some(at) => RENEWAL.pass(format!("renews {period} at {at}")),
		},
	}
}
