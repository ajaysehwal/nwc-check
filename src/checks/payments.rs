use serde_json::{Value, json};

use super::{Context, OVER_BUDGET, Payments, describe, over_budget, tracked};
use crate::check::{Check, Level, Outcome, Spec};
use crate::counterparty::{PayError, from_hex, sha256};
use crate::wallet::{Reply, Request, WalletError};

const MEMO: &str = "nwc-check";

const RECEIVE: Check = Check::new(
	"PAY-01",
	Level::Must,
	Spec::Nip47,
	"make_invoice returns a payable invoice",
	"Users share this invoice to get paid; if it can't be paid, the payment is lost.",
);
const SEND: Check = Check::new(
	"PAY-02",
	Level::Must,
	Spec::Nip47,
	"pay_invoice delivers the payment",
	"Apps report a payment as sent based on this reply.",
);
const PREIMAGE: Check = Check::new(
	"PAY-03",
	Level::Safety,
	Spec::Nip47,
	"pay_invoice returns the matching preimage",
	"The preimage is the user's proof that the invoice was paid.",
);
const FEES: Check = Check::new(
	"CORE-07",
	Level::Should,
	Spec::Nip47,
	"pay_invoice reports plausible fees_paid",
	"Apps show users what a payment cost from this field.",
);
const AMOUNTLESS: Check = Check::new(
	"CORE-08",
	Level::Must,
	Spec::Nip47,
	"pay_invoice pays the amount given for an amountless invoice",
	"Paying any other amount loses money or underpays the recipient.",
);

pub const CHECKS: &[&Check] = &[&RECEIVE, &SEND, &PREIMAGE, &FEES, &AMOUNTLESS];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return CHECKS.iter().map(|check| check.skip(reason)).collect(),
	};
	let mut outcomes = vec![tracked(receive(ctx, payments)).await];
	outcomes.extend(tracked(send(ctx, payments)).await);
	outcomes.push(tracked(amountless(ctx, payments)).await);
	outcomes
}

async fn receive(ctx: &Context, payments: &Payments) -> Outcome {
	if !ctx.supports("make_invoice") {
		return RECEIVE.skip("make_invoice not advertised");
	}
	let params = json!({ "amount": payments.amount_msat, "description": MEMO });
	let reply =
		ctx.wallet.send(Request::new("make_invoice", ctx.encryption()).params(params)).await;
	let invoice = match &reply {
		Ok(reply) if reply.error().is_none() => {
			reply.result().and_then(|result| result.get("invoice")).and_then(Value::as_str)
		}
		_ => return RECEIVE.fail(describe(&reply)),
	};
	let Some(invoice) = invoice else {
		return RECEIVE.fail("reply has no invoice");
	};
	if let Err(reason) = payments.reserve(1) {
		return RECEIVE.skip(reason);
	}
	match payments.node.pay(invoice).await {
		Ok(()) => RECEIVE.pass(format!("counterparty paid {}", sats(payments.amount_msat))),
		Err(PayError::Unable(reason)) => {
			RECEIVE.skip(format!("counterparty couldn't pay: {reason}"))
		}
		Err(PayError::Failed(error)) => RECEIVE.fail(format!("payment failed: {error}")),
	}
}

async fn send(ctx: &Context, payments: &Payments) -> [Outcome; 3] {
	let skip_all = |reason: &str| [SEND.skip(reason), PREIMAGE.skip(reason), FEES.skip(reason)];
	if !ctx.supports("pay_invoice") {
		return skip_all("pay_invoice not advertised");
	}
	let invoice = match payments.node.create_invoice(payments.amount_msat, MEMO).await {
		Ok(invoice) => invoice,
		Err(error) => {
			return skip_all(&format!("counterparty couldn't create an invoice: {error:#}"));
		}
	};
	if let Err(reason) = payments.reserve(1) {
		return skip_all(&reason);
	}

	let params = json!({ "invoice": invoice.bolt11 });
	let reply = ctx.wallet.send(Request::new("pay_invoice", ctx.encryption()).params(params)).await;
	if over_budget(&reply) {
		return skip_all(OVER_BUDGET);
	}
	let sent = match payments.node.paid_msat(&invoice.payment_hash).await {
		Ok(Some(_)) => SEND.pass(format!("counterparty received {}", sats(payments.amount_msat))),
		Ok(None) => SEND.fail(format!("invoice is unpaid, wallet {}", describe(&reply))),
		Err(error) => SEND.skip(format!("couldn't look up the invoice: {error:#}")),
	};

	let proof = match result_field(&reply, "preimage").and_then(Value::as_str) {
		None => PREIMAGE.fail(format!("no preimage, wallet {}", describe(&reply))),
		Some(preimage) => match from_hex(preimage) {
			Some(bytes) if sha256(&bytes) == invoice.payment_hash => {
				PREIMAGE.pass("matches the payment hash")
			}
			Some(_) => PREIMAGE.fail("doesn't match the payment hash"),
			None => PREIMAGE.fail(format!("not hex: {preimage}")),
		},
	};
	[sent, proof, fees(&reply, payments.amount_msat)]
}

fn fees(reply: &Result<Reply, WalletError>, amount_msat: u64) -> Outcome {
	if reply.as_ref().map_or(true, |reply| reply.error().is_some()) {
		return FEES.skip(format!("payment failed: {}", describe(reply)));
	}
	match result_field(reply, "fees_paid") {
		None => FEES.fail("reply has no fees_paid"),
		Some(fees) => match fees.as_u64() {
			Some(fees) if fees <= amount_msat => FEES.pass(format!("{fees} msats")),
			Some(fees) => {
				FEES.fail(format!("{fees} msats, more than the {amount_msat} msat payment"))
			}
			None => FEES.fail(format!("fees_paid isn't a whole number of msats: {fees}")),
		},
	}
}

async fn amountless(ctx: &Context, payments: &Payments) -> Outcome {
	if !ctx.supports("pay_invoice") {
		return AMOUNTLESS.skip("pay_invoice not advertised");
	}
	let invoice = match payments.node.create_invoice(0, MEMO).await {
		Ok(invoice) => invoice,
		Err(error) => {
			return AMOUNTLESS.skip(format!("counterparty couldn't create an invoice: {error:#}"));
		}
	};
	if let Err(reason) = payments.reserve(1) {
		return AMOUNTLESS.skip(reason);
	}
	let params = json!({ "invoice": invoice.bolt11, "amount": payments.amount_msat });
	let reply = ctx.wallet.send(Request::new("pay_invoice", ctx.encryption()).params(params)).await;
	if over_budget(&reply) {
		return AMOUNTLESS.skip(OVER_BUDGET);
	}
	match payments.node.paid_msat(&invoice.payment_hash).await {
		Ok(Some(paid)) if paid == payments.amount_msat => {
			AMOUNTLESS.pass(format!("counterparty received {}", sats(paid)))
		}
		Ok(Some(paid)) => AMOUNTLESS.fail(format!(
			"counterparty received {paid} msats, asked to pay {}",
			payments.amount_msat
		)),
		Ok(None) => AMOUNTLESS.fail(format!("invoice is unpaid, wallet {}", describe(&reply))),
		Err(error) => AMOUNTLESS.skip(format!("couldn't look up the invoice: {error:#}")),
	}
}

pub(super) fn result_field<'a>(
	reply: &'a Result<Reply, WalletError>,
	name: &str,
) -> Option<&'a Value> {
	reply.as_ref().ok()?.result()?.get(name)
}

fn sats(msat: u64) -> String {
	format!("{} sats", msat / 1000)
}
