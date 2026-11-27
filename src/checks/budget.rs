use futures::future::join_all;
use nostr_sdk::prelude::Keys;
use serde_json::json;

use super::{Context, Payments, describe, tracked};
use crate::check::{Check, Level, Outcome, Severity, Spec};
use crate::counterparty::{sha256, to_hex};
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

const EVERY_METHOD: Check = Check::new(
	"SAFE-08",
	Level::Safety,
	Severity::Critical,
	Spec::Safety,
	"Every spending method respects the budget",
	"A budget enforced only on pay_invoice can be bypassed with keysend or batch payments.",
);

pub const CHECKS: &[&Check] = &[&RACE, &EVERY_METHOD];

const SPENDING: [&str; 4] =
	["pay_invoice", "pay_keysend", "multi_pay_invoice", "multi_pay_keysend"];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	// One after the other: SAFE-08 tests against whatever budget SAFE-02 leaves.
	let race = tracked(race(ctx)).await;
	let every_method = tracked(every_method(ctx)).await;
	vec![race, every_method]
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

enum Probe {
	Refused,
	Paid(u64),
	Unclear(String),
}

async fn every_method(ctx: &Context) -> Outcome {
	let methods: Vec<&str> = SPENDING.into_iter().filter(|method| ctx.supports(method)).collect();
	if methods.is_empty() {
		return EVERY_METHOD.skip("no spending method advertised");
	}
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return EVERY_METHOD.skip(reason),
	};
	let remaining = match ctx.remaining_budget().await {
		Ok(Some(remaining)) => remaining,
		Ok(None) => return EVERY_METHOD.skip("no budget known (see --budget-sats)"),
		Err(error) => return EVERY_METHOD.skip(format!("get_budget: {error}")),
	};

	// A whole sat more than is left, so a wallet that checks the budget has to refuse.
	let amount = (remaining / 1000 + 1) * 1000;
	let total = amount * methods.len() as u64;
	if let Err(reason) = payments.reserve_msat(total) {
		return EVERY_METHOD.skip(format!("{reason}, needs {} sats", total / 1000));
	}

	let (mut refused, mut unclear) = (Vec::new(), Vec::new());
	for method in methods {
		match probe(ctx, payments, method, amount).await {
			Probe::Refused => refused.push(method),
			Probe::Paid(paid) => {
				return EVERY_METHOD.fail(format!(
					"{method} paid {} sats with only {} sats of budget left",
					paid / 1000,
					remaining / 1000
				));
			}
			Probe::Unclear(why) => unclear.push(format!("{method} {why}")),
		}
	}
	if unclear.is_empty() {
		EVERY_METHOD.pass(format!("{} refused with QUOTA_EXCEEDED", refused.join(", ")))
	} else {
		EVERY_METHOD
			.skip(format!("couldn't tell whether the budget applies: {}", unclear.join("; ")))
	}
}

/// Where a probe sends its payment.
enum Target {
	Invoice(String),
	Keysend { pubkey: String, preimage: String },
}

/// Asks `method` to pay `amount` to the counterparty, and reports whether the wallet refused on
/// budget, paid anyway, or failed for some other reason that proves nothing.
async fn probe(ctx: &Context, payments: &Payments, method: &str, amount: u64) -> Probe {
	let target = match method {
		"pay_invoice" | "multi_pay_invoice" => invoice_target(payments, amount).await,
		_ => keysend_target(payments).await,
	};
	let (target, payment_hash) = match target {
		Ok(target) => target,
		Err(error) => return Probe::Unclear(error),
	};
	let params = match (method, target) {
		("multi_pay_invoice", Target::Invoice(invoice)) => {
			json!({ "invoices": [{ "id": "nwc-check", "invoice": invoice }] })
		}
		(_, Target::Invoice(invoice)) => json!({ "invoice": invoice }),
		("multi_pay_keysend", Target::Keysend { pubkey, preimage }) => json!({
			"keysends": [{ "id": "nwc-check", "amount": amount, "pubkey": pubkey, "preimage": preimage }]
		}),
		(_, Target::Keysend { pubkey, preimage }) => {
			json!({ "amount": amount, "pubkey": pubkey, "preimage": preimage })
		}
	};
	let reply = ctx.wallet.send(Request::new(method, ctx.encryption()).params(params)).await;

	match payments.node.paid_msat(&payment_hash).await {
		Ok(Some(paid)) => Probe::Paid(paid),
		Ok(None) => match &reply {
			Ok(reply) if reply.error_code() == Some("QUOTA_EXCEEDED") => Probe::Refused,
			other => Probe::Unclear(describe(other)),
		},
		Err(error) => Probe::Unclear(format!("couldn't look up the payment: {error:#}")),
	}
}

async fn invoice_target(payments: &Payments, amount: u64) -> Result<(Target, [u8; 32]), String> {
	let invoice = payments
		.node
		.create_invoice(amount, "nwc-check")
		.await
		.map_err(|error| format!("counterparty couldn't create an invoice: {error:#}"))?;
	Ok((Target::Invoice(invoice.bolt11), invoice.payment_hash))
}

/// A keysend to the counterparty with a preimage we pick, so its payment hash is known up front.
async fn keysend_target(payments: &Payments) -> Result<(Target, [u8; 32]), String> {
	let info = payments
		.node
		.info()
		.await
		.map_err(|error| format!("couldn't read the counterparty's pubkey: {error:#}"))?;
	let preimage = Keys::generate().secret_key().to_secret_bytes();
	let target = Target::Keysend { pubkey: info.pubkey, preimage: to_hex(&preimage) };
	Ok((target, sha256(&preimage)))
}
