use serde_json::{Value, json};

use super::payments::result_field;
use super::{Context, Payments, describe};
use crate::check::{Check, Level, Outcome, Severity, Spec};
use crate::evidence::track;
use crate::wallet::Request;

const BALANCE: Check = Check::new(
	"CORE-09",
	Level::Safety,
	Severity::High,
	Spec::Nip47,
	"Balance drops by the amount plus fees_paid",
	"A wallet that charges more than it reports takes money the user can't see.",
);
const BUDGET_USE: Check = Check::new(
	"EXT13-04",
	Level::Safety,
	Severity::High,
	Spec::Nwc13,
	"used_budget rises by the amount plus fees_paid",
	"A budget that leaves out fees lets a connection spend more than the user allowed.",
);

pub const CHECKS: &[&Check] = &[&BALANCE, &BUDGET_USE];

type Reading = Option<Result<Option<u64>, String>>;

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let skip_all = |reason: &str| vec![BALANCE.skip(reason), BUDGET_USE.skip(reason)];
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return skip_all(reason),
	};
	if !ctx.supports("pay_invoice") {
		return skip_all("pay_invoice not advertised");
	}
	let (paid, evidence) = track(pay_once(ctx, payments)).await;
	let outcomes = match paid {
		Ok(Paid { fees, balance, budget }) => {
			let expected = payments.amount_msat + fees;
			vec![
				compare(
					&BALANCE,
					"get_balance",
					"reply has no balance",
					balance,
					expected,
					fees,
					|before, after| before.saturating_sub(after),
				),
				compare(
					&BUDGET_USE,
					"get_budget",
					"no budget configured",
					budget,
					expected,
					fees,
					|before, after| after.saturating_sub(before),
				),
			]
		}
		Err(reason) => skip_all(&reason),
	};
	outcomes.into_iter().map(|outcome| outcome.with(&evidence)).collect()
}

struct Paid {
	fees: u64,
	balance: (Reading, Reading),
	budget: (Reading, Reading),
}

async fn pay_once(ctx: &Context, payments: &Payments) -> Result<Paid, String> {
	if !ctx.supports("get_balance") && !ctx.supports("get_budget") {
		return Err("neither get_balance nor get_budget advertised".to_owned());
	}
	let invoice = payments
		.node
		.create_invoice(payments.amount_msat, "nwc-check")
		.await
		.map_err(|error| format!("counterparty couldn't create an invoice: {error:#}"))?;
	payments.reserve(1)?;

	let before = read(ctx).await;
	let params = json!({ "invoice": invoice.bolt11 });
	let reply = ctx.wallet.send(Request::new("pay_invoice", ctx.encryption()).params(params)).await;
	if reply.as_ref().map_or(true, |reply| reply.error().is_some()) {
		return Err(format!("payment failed: {}", describe(&reply)));
	}
	let fees = result_field(&reply, "fees_paid")
		.and_then(Value::as_u64)
		.ok_or("no fees_paid to compare against")?;
	let after = read(ctx).await;
	Ok(Paid { fees, balance: (before.0, after.0), budget: (before.1, after.1) })
}

async fn read(ctx: &Context) -> (Reading, Reading) {
	let balance = async {
		if ctx.supports("get_balance") {
			Some(number(ctx, "get_balance", "balance").await)
		} else {
			None
		}
	};
	let budget = async {
		if ctx.supports("get_budget") {
			Some(number(ctx, "get_budget", "used_budget").await)
		} else {
			None
		}
	};
	tokio::join!(balance, budget)
}

fn compare(
	check: &Check,
	method: &str,
	missing: &str,
	(before, after): (Reading, Reading),
	expected: u64,
	fees: u64,
	change: impl Fn(u64, u64) -> u64,
) -> Outcome {
	let (before, after) = match (before, after) {
		(None, _) | (_, None) => return check.skip(format!("{method} not advertised")),
		(Some(Err(error)), _) | (_, Some(Err(error))) => {
			return check.skip(format!("{method}: {error}"));
		}
		(Some(Ok(before)), Some(Ok(after))) => (before, after),
	};
	let (Some(before), Some(after)) = (before, after) else {
		return check.skip(missing);
	};
	let changed = change(before, after);
	let amount = expected - fees;
	// Wallets that keep figures in whole sats round each one by up to 1 sat.
	if changed.abs_diff(expected) < 1000 {
		check.pass(format!("changed by {changed} msats, {amount} plus {fees} fees"))
	} else {
		check.fail(format!("changed by {changed} msats, expected {amount} plus {fees} fees"))
	}
}

async fn number(ctx: &Context, method: &str, field: &str) -> Result<Option<u64>, String> {
	let reply = ctx.wallet.send(Request::new(method, ctx.encryption())).await;
	match &reply {
		Ok(reply) if reply.error().is_none() => {
			Ok(reply.result().and_then(|result| result.get(field)).and_then(Value::as_u64))
		}
		_ => Err(describe(&reply)),
	}
}
