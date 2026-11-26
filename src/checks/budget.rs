use futures::future::join_all;
use serde_json::json;

use super::{Context, describe, tracked};
use crate::check::{Check, Level, Outcome, Severity, Spec};
use crate::wallet::Request;

const PAYMENTS: u64 = 5;

const RACE: Check = Check::new(
	"SAFE-02",
	Level::Safety,
	Severity::Critical,
	Spec::Safety,
	"Concurrent payments stay within the budget",
	"A wallet that checks the budget before reserving it can overspend when payments arrive together.",
);

pub const CHECKS: &[&Check] = &[&RACE];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	vec![tracked(race(ctx)).await]
}

async fn race(ctx: &Context) -> Outcome {
	if !ctx.supports("pay_invoice") {
		return RACE.skip("pay_invoice not advertised");
	}
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return RACE.skip(reason),
	};
	let budget = match ctx.remaining_budget().await {
		Ok(Some(0)) => return RACE.skip("the budget is used up"),
		Ok(Some(budget)) => budget,
		Ok(None) => return RACE.skip("no budget known (see --budget-sats)"),
		Err(error) => return RACE.skip(format!("get_budget: {error}")),
	};

	// Sized so that one payment fewer fits the budget and all of them go over it.
	let each = budget.div_ceil(PAYMENTS - 1).div_ceil(1000) * 1000;
	if let Err(reason) = payments.reserve_msat(each * PAYMENTS) {
		return RACE.skip(format!("{reason}, needs {} sats", each * PAYMENTS / 1000));
	}
	let invoices = join_all((0..PAYMENTS).map(|_| payments.node.create_invoice(each, "nwc-check")))
		.await
		.into_iter()
		.collect::<Result<Vec<_>, _>>();
	let invoices = match invoices {
		Ok(invoices) => invoices,
		Err(error) => {
			return RACE.skip(format!("counterparty couldn't create invoices: {error:#}"));
		}
	};

	let signed = invoices
		.iter()
		.map(|invoice| {
			let params = json!({ "invoice": invoice.bolt11 });
			ctx.wallet.sign(Request::new("pay_invoice", ctx.encryption()).params(params))
		})
		.collect::<Result<Vec<_>, _>>();
	let signed = match signed {
		Ok(signed) => signed,
		Err(error) => return RACE.skip(error.to_string()),
	};
	let replies = join_all(signed.iter().map(|signed| ctx.wallet.deliver(signed))).await;

	let received =
		join_all(invoices.iter().map(|invoice| payments.node.paid_msat(&invoice.payment_hash)))
			.await;
	let (mut paid, mut paid_msat) = (0, 0);
	for result in received {
		match result {
			Ok(Some(amount_msat)) => (paid, paid_msat) = (paid + 1, paid_msat + amount_msat),
			Ok(None) => {}
			Err(error) => return RACE.skip(format!("couldn't look up an invoice: {error:#}")),
		}
	}

	let refused = replies.iter().filter(|reply| !matches!(reply, Ok(r) if r.error().is_none()));
	let other_codes: Vec<String> = refused
		.clone()
		.filter(|reply| !matches!(reply, Ok(r) if r.error_code() == Some("QUOTA_EXCEEDED")))
		.map(describe)
		.collect();
	let summary = format!(
		"paid {paid} of {PAYMENTS} payments of {} sats against a {} sat budget",
		each / 1000,
		budget / 1000
	);
	if paid_msat > budget {
		RACE.fail(summary)
	} else if let Some(first) = other_codes.first() {
		RACE.pass(format!("{summary}; a refusal {first} instead of QUOTA_EXCEEDED"))
	} else {
		RACE.pass(format!("{summary}, {} refused", refused.count()))
	}
}
