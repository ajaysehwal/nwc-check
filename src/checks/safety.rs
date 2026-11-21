use nostr_sdk::prelude::Timestamp;
use serde_json::{Value, json};

use super::{Context, describe, state};
use crate::check::{Check, Level, Outcome};
use crate::counterparty::to_hex;
use crate::wallet::{Request, WalletError};

const EXPIRED: Check = Check::new(
	"SAFE-01",
	Level::Safety,
	"Expired requests are ignored",
	"An expired payment request could be paid long after the user gave up on it.",
);
const DUPLICATE: Check = Check::new(
	"SAFE-03",
	Level::Safety,
	"A duplicated pay_invoice is paid once",
	"Relays can deliver the same request twice, and a wallet that pays every copy pays twice.",
);

const READONLY: Check = Check::new(
	"SAFE-05",
	Level::Safety,
	"A read-only connection can't pay",
	"A connection the user granted no payment permission must never spend their funds.",
);

pub const CHECKS: &[&Check] = &[&EXPIRED, &DUPLICATE, &READONLY];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let (expired, duplicate, readonly) = tokio::join!(expired(ctx), duplicate(ctx), readonly(ctx));
	vec![expired, duplicate, readonly]
}

async fn expired(ctx: &Context) -> Outcome {
	let expired = Timestamp::from_secs(Timestamp::now().as_secs() - 60);
	let request = Request::new("get_info", ctx.encryption()).expiration(expired);
	match ctx.wallet.send(request).await {
		Err(WalletError::NoReply) => EXPIRED.pass("no reply"),
		Err(WalletError::Rejected(_)) => EXPIRED.skip("relays dropped the expired request"),
		Ok(reply) if reply.error().is_some() => EXPIRED.pass(describe(&Ok(reply))),
		other => EXPIRED.fail(format!("expired request was answered: {}", describe(&other))),
	}
}

async fn duplicate(ctx: &Context) -> Outcome {
	for method in ["pay_invoice", "list_transactions"] {
		if !ctx.supports(method) {
			return DUPLICATE.skip(format!("{method} not advertised"));
		}
	}
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return DUPLICATE.skip(reason),
	};
	// A wallet with this bug pays twice, so the budget must cover both payments.
	if let Err(reason) = payments.reserve(2) {
		return DUPLICATE.skip(reason);
	}
	let invoice = match payments.node.create_invoice(payments.amount_msat, "nwc-check").await {
		Ok(invoice) => invoice,
		Err(error) => {
			return DUPLICATE.skip(format!("counterparty couldn't create an invoice: {error:#}"));
		}
	};

	let request =
		Request::new("pay_invoice", ctx.encryption()).params(json!({ "invoice": invoice.bolt11 }));
	let signed = match ctx.wallet.sign(request) {
		Ok(signed) => signed,
		Err(error) => return DUPLICATE.skip(error.to_string()),
	};
	match ctx.wallet.deliver(&signed).await {
		Ok(reply) if reply.error().is_none() => {}
		other => return DUPLICATE.skip(format!("first payment failed: {}", describe(&other))),
	}
	let copy = match ctx.wallet.redeliver(&signed).await {
		Ok(copy) if copy.echoed => copy,
		Ok(_) => return DUPLICATE.skip("relays didn't pass the duplicate on"),
		Err(error) => return DUPLICATE.skip(format!("couldn't send the duplicate: {error}")),
	};

	match settled_payments(ctx, &to_hex(&invoice.payment_hash)).await {
		Ok(0) => DUPLICATE.skip("the payment isn't listed by list_transactions"),
		Ok(1) if copy.reply.is_some() => DUPLICATE.pass("paid once, the copy got a reply"),
		Ok(1) => DUPLICATE.pass("paid once, the copy was ignored"),
		Ok(count) => DUPLICATE.fail(format!("paid {count} times")),
		Err(error) => DUPLICATE.skip(format!("list_transactions: {error}")),
	}
}

async fn settled_payments(ctx: &Context, payment_hash: &str) -> Result<usize, String> {
	let request =
		Request::new("list_transactions", ctx.encryption()).params(json!({ "type": "outgoing" }));
	let reply = ctx.wallet.send(request).await;
	let transactions = match &reply {
		Ok(reply) if reply.error().is_none() => reply
			.result()
			.and_then(|result| result.get("transactions"))
			.and_then(Value::as_array)
			.ok_or("reply has no transactions")?,
		_ => return Err(describe(&reply)),
	};
	Ok(transactions
		.iter()
		.filter(|tx| tx.get("payment_hash").and_then(Value::as_str) == Some(payment_hash))
		.filter(|tx| state(tx) == "settled")
		.count())
}

async fn readonly(ctx: &Context) -> Outcome {
	let Some(readonly) = &ctx.readonly else {
		return READONLY.skip("no read-only connection (see --readonly-uri)");
	};
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return READONLY.skip(reason),
	};
	let invoice = match payments.node.create_invoice(payments.amount_msat, "nwc-check").await {
		Ok(invoice) => invoice,
		Err(error) => {
			return READONLY.skip(format!("counterparty couldn't create an invoice: {error:#}"));
		}
	};
	// A wallet that ignores permissions pays this invoice.
	if let Err(reason) = payments.reserve(1) {
		return READONLY.skip(reason);
	}
	let params = json!({ "invoice": invoice.bolt11 });
	let reply = readonly.send(Request::new("pay_invoice", ctx.encryption()).params(params)).await;
	match payments.node.paid_msat(&invoice.payment_hash).await {
		Ok(Some(_)) => READONLY.fail("the read-only connection paid the invoice"),
		Ok(None) => match &reply {
			Ok(reply) if reply.error_code() == Some("RESTRICTED") => {
				READONLY.pass("RESTRICTED, nothing paid")
			}
			other => {
				READONLY.fail(format!("nothing paid, but expected RESTRICTED, {}", describe(other)))
			}
		},
		Err(error) => READONLY.skip(format!("couldn't look up the invoice: {error:#}")),
	}
}
