use nostr_sdk::prelude::{Keys, Timestamp};
use serde_json::{Value, json};

use super::{Context, OVER_BUDGET, describe, over_budget, state, tracked};
use crate::check::{Check, Level, Outcome, Severity, Spec};
use crate::counterparty::to_hex;
use crate::wallet::{Reply, Request, WalletError};

const EXPIRED: Check = Check::new(
	"SAFE-01",
	Level::Safety,
	Severity::High,
	Spec::Nip40,
	"Expired requests are ignored",
	"An expired payment request could be paid long after the user gave up on it.",
);
const DUPLICATE: Check = Check::new(
	"SAFE-03",
	Level::Safety,
	Severity::Critical,
	Spec::Safety,
	"A duplicated pay_invoice is paid once",
	"Relays can deliver the same request twice, and a wallet that pays every copy pays twice.",
);

const STALE: Check = Check::new(
	"SAFE-06",
	Level::Safety,
	Severity::High,
	Spec::Safety,
	"A stale pay_invoice is not paid",
	"A relay, or anyone holding a captured request, can send it again long after it was signed, and a wallet with no age limit pays it again.",
);

/// How far back the stale request is signed. A wallet that allows for clocks a few minutes
/// off still has to refuse a request this old.
const STALE_AGE_SECONDS: u64 = 60 * 60;

const READONLY: Check = Check::new(
	"SAFE-05",
	Level::Safety,
	Severity::Critical,
	Spec::Nip47,
	"A read-only connection can't pay",
	"A connection the user granted no payment permission must never spend their funds.",
);

const OVERRIDE: Check = Check::new(
	"SAFE-04",
	Level::Safety,
	Severity::Critical,
	Spec::Safety,
	"amount can't change a fixed-amount invoice",
	"A wallet that pays the amount param instead of the invoice amount can be made to overpay.",
);

const ESCALATION: Check = Check::new(
	"SAFE-07",
	Level::Safety,
	Severity::Critical,
	Spec::Safety,
	"create_connection can't grant more than its creator has",
	"A connection that can mint a bigger budget or more methods than its own turns any leak into an unlimited spending key.",
);

pub const CHECKS: &[&Check] = &[&EXPIRED, &DUPLICATE, &OVERRIDE, &STALE, &READONLY, &ESCALATION];

/// Methods a new connection could be granted, in the order SAFE-07 looks for one the creator lacks.
const GRANTABLE: [&str; 10] = [
	"pay_invoice",
	"pay_keysend",
	"multi_pay_invoice",
	"multi_pay_keysend",
	"make_invoice",
	"lookup_invoice",
	"list_transactions",
	"get_balance",
	"get_budget",
	"sign_message",
];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let (expired, duplicate, overridden, stale, readonly, escalation) = tokio::join!(
		tracked(expired(ctx)),
		tracked(duplicate(ctx)),
		tracked(amount_override(ctx)),
		tracked(stale(ctx)),
		tracked(readonly(ctx)),
		tracked(escalation(ctx)),
	);
	vec![expired, duplicate, overridden, stale, readonly, escalation]
}

async fn expired(ctx: &Context) -> Outcome {
	let expired = Timestamp::from_secs(Timestamp::now().as_secs() - 60);
	let request = Request::new("get_info", ctx.encryption()).expiration(expired);
	match ctx.wallet.send(request).await {
		Err(WalletError::NoReply) => EXPIRED.pass("no reply"),
		Err(WalletError::NoETag) => {
			EXPIRED.skip("replies have no e tag, so they can't be told apart")
		}
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

async fn stale(ctx: &Context) -> Outcome {
	if !ctx.supports("pay_invoice") {
		return STALE.skip("pay_invoice not advertised");
	}
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return STALE.skip(reason),
	};
	let invoice = match payments.node.create_invoice(payments.amount_msat, "nwc-check").await {
		Ok(invoice) => invoice,
		Err(error) => {
			return STALE.skip(format!("counterparty couldn't create an invoice: {error:#}"));
		}
	};
	// A wallet without an age limit pays this invoice.
	if let Err(reason) = payments.reserve(1) {
		return STALE.skip(reason);
	}
	let signed_at = Timestamp::from_secs(Timestamp::now().as_secs() - STALE_AGE_SECONDS);
	let request = Request::new("pay_invoice", ctx.encryption())
		.params(json!({ "invoice": invoice.bolt11 }))
		.created_at(signed_at);
	let reply = ctx.wallet.send(request).await;
	if over_budget(&reply) {
		return STALE.skip(OVER_BUDGET);
	}
	match payments.node.paid_msat(&invoice.payment_hash).await {
		Ok(Some(_)) => STALE.fail("paid a request signed an hour ago"),
		Ok(None) => match &reply {
			Err(WalletError::NoReply) => STALE.pass("no reply, nothing paid"),
			Err(WalletError::Rejected(_)) => STALE.skip("relays dropped the stale request"),
			Ok(answer) if answer.error().is_some() => STALE.pass(describe(&reply)),
			other => STALE.skip(format!("nothing arrived, but the wallet {}", describe(other))),
		},
		Err(error) => STALE.skip(format!("couldn't look up the invoice: {error:#}")),
	}
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

async fn amount_override(ctx: &Context) -> Outcome {
	if !ctx.supports("pay_invoice") {
		return OVERRIDE.skip("pay_invoice not advertised");
	}
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return OVERRIDE.skip(reason),
	};
	let invoice = match payments.node.create_invoice(payments.amount_msat, "nwc-check").await {
		Ok(invoice) => invoice,
		Err(error) => {
			return OVERRIDE.skip(format!("counterparty couldn't create an invoice: {error:#}"));
		}
	};
	// LND accepts up to twice the invoice amount, so a wallet that honours the override overpays.
	let overridden = payments.amount_msat * 2;
	if let Err(reason) = payments.reserve(2) {
		return OVERRIDE.skip(reason);
	}
	let params = json!({ "invoice": invoice.bolt11, "amount": overridden });
	let reply = ctx.wallet.send(Request::new("pay_invoice", ctx.encryption()).params(params)).await;
	if over_budget(&reply) {
		return OVERRIDE.skip(OVER_BUDGET);
	}
	let rejected = reply.as_ref().is_ok_and(|reply| reply.error().is_some());
	match payments.node.paid_msat(&invoice.payment_hash).await {
		Ok(Some(paid)) if paid == payments.amount_msat => {
			OVERRIDE.pass("paid the invoice amount and ignored the amount param")
		}
		Ok(Some(paid)) => {
			OVERRIDE.fail(format!("paid {paid} msats for a {} msat invoice", payments.amount_msat))
		}
		Ok(None) if rejected => OVERRIDE.pass(format!("rejected, the wallet {}", describe(&reply))),
		Ok(None) => OVERRIDE.skip(format!("nothing arrived, but the wallet {}", describe(&reply))),
		Err(error) => OVERRIDE.skip(format!("couldn't look up the invoice: {error:#}")),
	}
}

async fn escalation(ctx: &Context) -> Outcome {
	if !ctx.supports("create_connection") {
		return ESCALATION.skip("create_connection not advertised");
	}
	let budget = match ctx.remaining_budget().await {
		Ok(budget) => budget.filter(|_| ctx.supports("pay_invoice")),
		Err(error) => return ESCALATION.skip(format!("get_budget: {error}")),
	};
	let missing = GRANTABLE.into_iter().find(|method| !ctx.supports(method));

	let mut asked = Vec::new();
	if let Some(remaining) = budget {
		// A whole sat over, so a wallet that rounds to sats can't hide the difference.
		let max_amount = remaining + 1000;
		let params = json!({ "request_methods": ["pay_invoice"], "max_amount": max_amount });
		let what =
			format!("a {} sat budget with {} sats left", max_amount / 1000, remaining / 1000);
		asked.push((what, create_connection(ctx, params).await));
	}
	if let Some(method) = missing {
		let params = json!({ "request_methods": ["get_info", method], "max_amount": 1000 });
		asked.push((format!("{method}, which it lacks"), create_connection(ctx, params).await));
	}
	if asked.is_empty() {
		return ESCALATION
			.skip("the connection has no budget and every method, so it can't be exceeded");
	}

	let mut refused = Vec::new();
	for (what, reply) in &asked {
		match reply {
			Ok(reply) if reply.error().is_some() => refused.push(what.as_str()),
			Ok(reply) => {
				let wallet = reply
					.result()
					.and_then(|result| result.get("wallet_pubkey"))
					.and_then(Value::as_str)
					.unwrap_or("unknown");
				return ESCALATION.fail(format!(
					"granted {what}; delete the connection with wallet pubkey {wallet}"
				));
			}
			Err(error) => return ESCALATION.skip(format!("asking for {what}: {error}")),
		}
	}
	ESCALATION.pass(format!("refused {}", refused.join(" and ")))
}

/// Asks for a new connection for a client key that is thrown away, so even a connection
/// the wallet wrongly creates can't be used.
async fn create_connection(ctx: &Context, mut params: Value) -> Result<Reply, WalletError> {
	params["pubkey"] = json!(Keys::generate().public_key().to_hex());
	params["name"] = json!("nwc-check SAFE-07 probe, safe to delete");
	params["budget_renewal"] = json!("never");
	ctx.wallet.send(Request::new("create_connection", ctx.encryption()).params(params)).await
}
