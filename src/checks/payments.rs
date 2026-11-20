use serde_json::{Value, json};

use super::{Context, Payments, describe};
use crate::check::{Check, Level, Outcome};
use crate::counterparty::{PayError, from_hex, sha256};
use crate::wallet::Request;

const MEMO: &str = "nwc-check";

const RECEIVE: Check = Check::new(
	"PAY-01",
	Level::Must,
	"make_invoice returns a payable invoice",
	"Users share this invoice to get paid; if it can't be paid, the payment is lost.",
);
const SEND: Check = Check::new(
	"PAY-02",
	Level::Must,
	"pay_invoice delivers the payment",
	"Apps report a payment as sent based on this reply.",
);
const PREIMAGE: Check = Check::new(
	"PAY-03",
	Level::Safety,
	"pay_invoice returns the matching preimage",
	"The preimage is the user's proof that the invoice was paid.",
);

pub const CHECKS: &[&Check] = &[&RECEIVE, &SEND, &PREIMAGE];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return CHECKS.iter().map(|check| check.skip(reason)).collect(),
	};
	let mut outcomes = vec![receive(ctx, payments).await];
	outcomes.extend(send(ctx, payments).await);
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
	if let Err(reason) = payments.reserve() {
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

async fn send(ctx: &Context, payments: &Payments) -> [Outcome; 2] {
	if !ctx.supports("pay_invoice") {
		return [
			SEND.skip("pay_invoice not advertised"),
			PREIMAGE.skip("pay_invoice not advertised"),
		];
	}
	let invoice = match payments.node.create_invoice(payments.amount_msat, MEMO).await {
		Ok(invoice) => invoice,
		Err(error) => {
			let reason = format!("counterparty couldn't create an invoice: {error:#}");
			return [SEND.skip(&reason), PREIMAGE.skip(reason)];
		}
	};

	if let Err(reason) = payments.reserve() {
		return [SEND.skip(&reason), PREIMAGE.skip(reason)];
	}
	let params = json!({ "invoice": invoice.bolt11 });
	let reply = ctx.wallet.send(Request::new("pay_invoice", ctx.encryption()).params(params)).await;
	let sent = match payments.node.is_settled(&invoice.payment_hash).await {
		Ok(true) => SEND.pass(format!("counterparty received {}", sats(payments.amount_msat))),
		Ok(false) => SEND.fail(format!("invoice is unpaid, wallet {}", describe(&reply))),
		Err(error) => SEND.skip(format!("couldn't look up the invoice: {error:#}")),
	};

	let preimage = reply
		.as_ref()
		.ok()
		.and_then(|reply| reply.result())
		.and_then(|result| result.get("preimage"))
		.and_then(Value::as_str);
	let proof = match preimage {
		None => PREIMAGE.fail(format!("no preimage, wallet {}", describe(&reply))),
		Some(preimage) => match from_hex(preimage) {
			Some(bytes) if sha256(&bytes) == invoice.payment_hash => {
				PREIMAGE.pass("matches the payment hash")
			}
			Some(_) => PREIMAGE.fail("doesn't match the payment hash"),
			None => PREIMAGE.fail(format!("not hex: {preimage}")),
		},
	};
	[sent, proof]
}

fn sats(msat: u64) -> String {
	format!("{} sats", msat / 1000)
}
