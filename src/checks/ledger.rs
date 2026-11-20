use serde_json::{Value, json};

use super::payments::result_field;
use super::{Context, Payments, describe};
use crate::check::{Check, Level, Outcome};
use crate::wallet::Request;

const BALANCE: Check = Check::new(
	"CORE-09",
	Level::Safety,
	"Balance drops by the amount plus fees_paid",
	"A wallet that charges more than it reports takes money the user can't see.",
);

pub const CHECKS: &[&Check] = &[&BALANCE];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return vec![BALANCE.skip(reason)],
	};
	vec![balance(ctx, payments).await]
}

async fn balance(ctx: &Context, payments: &Payments) -> Outcome {
	for method in ["pay_invoice", "get_balance"] {
		if !ctx.supports(method) {
			return BALANCE.skip(format!("{method} not advertised"));
		}
	}
	let invoice = match payments.node.create_invoice(payments.amount_msat, "nwc-check").await {
		Ok(invoice) => invoice,
		Err(error) => {
			return BALANCE.skip(format!("counterparty couldn't create an invoice: {error:#}"));
		}
	};
	if let Err(reason) = payments.reserve(1) {
		return BALANCE.skip(reason);
	}

	let before = match get_balance(ctx).await {
		Ok(balance) => balance,
		Err(error) => return BALANCE.skip(format!("get_balance: {error}")),
	};
	let params = json!({ "invoice": invoice.bolt11 });
	let reply = ctx.wallet.send(Request::new("pay_invoice", ctx.encryption()).params(params)).await;
	if reply.as_ref().map_or(true, |reply| reply.error().is_some()) {
		return BALANCE.skip(format!("payment failed: {}", describe(&reply)));
	}
	let Some(fees) = result_field(&reply, "fees_paid").and_then(Value::as_u64) else {
		return BALANCE.skip("no fees_paid to compare against");
	};
	let after = match get_balance(ctx).await {
		Ok(balance) => balance,
		Err(error) => return BALANCE.skip(format!("get_balance: {error}")),
	};

	let expected = payments.amount_msat + fees;
	let dropped = before.saturating_sub(after);
	// Wallets that keep balances in whole sats round each figure by up to 1 sat.
	if dropped.abs_diff(expected) < 1000 {
		BALANCE
			.pass(format!("dropped by {dropped} msats, {} plus {fees} fees", payments.amount_msat))
	} else {
		BALANCE.fail(format!(
			"dropped by {dropped} msats, expected {} plus {fees} fees",
			payments.amount_msat
		))
	}
}

pub(super) async fn get_balance(ctx: &Context) -> Result<u64, String> {
	let reply = ctx.wallet.send(Request::new("get_balance", ctx.encryption())).await;
	match &reply {
		Ok(reply) if reply.error().is_none() => reply
			.result()
			.and_then(|result| result.get("balance"))
			.and_then(Value::as_u64)
			.ok_or_else(|| "reply has no balance".to_owned()),
		_ => Err(describe(&reply)),
	}
}
