use std::str::FromStr;
use std::time::Duration;

use lightning_invoice::{Bolt11Invoice, Bolt11InvoiceDescriptionRef};
use nostr_sdk::prelude::Keys;
use serde_json::{Value, json};

use super::{Context, describe};
use crate::check::{Check, Level, Outcome};
use crate::counterparty::{sha256, to_hex};
use crate::wallet::Request;

const AMOUNT_MSAT: u64 = 21_000;
const DESCRIPTION: &str = "nwc-check";
const EXPIRY_SECS: u64 = 3600;

const CREATE: Check = Check::new(
	"CORE-01",
	Level::Must,
	"make_invoice matches amount, description and expiry",
	"A wrong amount or description makes the payer pay something other than what was asked.",
);
const DESCRIPTION_HASH: Check = Check::new(
	"CORE-02",
	Level::Must,
	"make_invoice sets the description_hash",
	"Services like LNURL rely on the hash to bind the invoice to their request.",
);
const LOOKUP: Check = Check::new(
	"CORE-03",
	Level::Must,
	"lookup_invoice finds by payment_hash and invoice",
	"Apps look invoices up either way to track their status.",
);
const SETTLE: Check = Check::new(
	"CORE-04",
	Level::Must,
	"lookup_invoice shows pending, then settled",
	"Apps tell users they were paid from this state.",
);
const UNKNOWN: Check = Check::new(
	"CORE-05",
	Level::Must,
	"Unknown invoice returns NOT_FOUND",
	"Clients need NOT_FOUND to tell a missing invoice from a failure.",
);

pub const CHECKS: &[&Check] = &[&CREATE, &DESCRIPTION_HASH, &LOOKUP, &SETTLE, &UNKNOWN];

struct Created {
	invoice: Bolt11Invoice,
	result: Value,
}

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	if !ctx.supports("make_invoice") {
		let reason = "make_invoice not advertised";
		let mut outcomes: Vec<Outcome> =
			[&CREATE, &DESCRIPTION_HASH, &LOOKUP, &SETTLE].iter().map(|c| c.skip(reason)).collect();
		outcomes.push(unknown(ctx).await);
		return outcomes;
	}

	let hash = sha256(b"nwc-check description");
	let (created, hashed, settle, unknown) = tokio::join!(
		make_invoice(
			ctx,
			json!({ "amount": AMOUNT_MSAT, "description": DESCRIPTION, "expiry": EXPIRY_SECS })
		),
		make_invoice(ctx, json!({ "amount": AMOUNT_MSAT, "description_hash": to_hex(&hash) })),
		settle(ctx),
		unknown(ctx),
	);
	vec![
		create(&created),
		description_hash(&hashed, &hash),
		lookup(ctx, &created).await,
		settle,
		unknown,
	]
}

async fn make_invoice(ctx: &Context, params: Value) -> Result<Created, String> {
	let reply =
		ctx.wallet.send(Request::new("make_invoice", ctx.encryption()).params(params)).await;
	let result = match &reply {
		Ok(reply) if reply.error().is_none() => reply.result().cloned(),
		_ => return Err(describe(&reply)),
	}
	.ok_or("reply has no result")?;
	let invoice = result.get("invoice").and_then(Value::as_str).ok_or("reply has no invoice")?;
	let invoice =
		Bolt11Invoice::from_str(invoice).map_err(|error| format!("invalid invoice: {error}"))?;
	Ok(Created { invoice, result })
}

fn create(created: &Result<Created, String>) -> Outcome {
	let Created { invoice, result } = match created {
		Ok(created) => created,
		Err(error) => return CREATE.fail(error),
	};
	let mut problems = Vec::new();
	if invoice.amount_milli_satoshis() != Some(AMOUNT_MSAT) {
		problems.push(format!(
			"invoice is for {:?} msats, asked for {AMOUNT_MSAT}",
			invoice.amount_milli_satoshis()
		));
	}
	match invoice.description() {
		Bolt11InvoiceDescriptionRef::Direct(description)
			if description.as_inner().0 == DESCRIPTION => {}
		_ => problems.push(format!("invoice description isn't \"{DESCRIPTION}\"")),
	}
	if invoice.expiry_time().as_secs() != EXPIRY_SECS {
		problems.push(format!(
			"invoice expires after {}s, asked for {EXPIRY_SECS}s",
			invoice.expiry_time().as_secs()
		));
	}
	if result
		.get("payment_hash")
		.and_then(Value::as_str)
		.is_some_and(|hash| hash != to_hex(invoice.payment_hash().as_ref()))
	{
		problems.push("payment_hash doesn't match the invoice".to_owned());
	}
	if problems.is_empty() {
		CREATE.pass(format!("{} sats, description and expiry match", AMOUNT_MSAT / 1000))
	} else {
		CREATE.fail(problems.join("; "))
	}
}

fn description_hash(created: &Result<Created, String>, expected: &[u8; 32]) -> Outcome {
	match created {
		Ok(Created { invoice, .. }) => match invoice.description() {
			Bolt11InvoiceDescriptionRef::Hash(hash)
				if AsRef::<[u8]>::as_ref(&hash.0) == expected =>
			{
				DESCRIPTION_HASH.pass("invoice carries the hash")
			}
			Bolt11InvoiceDescriptionRef::Hash(_) => {
				DESCRIPTION_HASH.fail("invoice carries a different hash")
			}
			Bolt11InvoiceDescriptionRef::Direct(_) => {
				DESCRIPTION_HASH.fail("invoice has a plain description instead of the hash")
			}
		},
		Err(error) => DESCRIPTION_HASH.fail(error),
	}
}

async fn lookup(ctx: &Context, created: &Result<Created, String>) -> Outcome {
	if !ctx.supports("lookup_invoice") {
		return LOOKUP.skip("lookup_invoice not advertised");
	}
	let Ok(Created { invoice, .. }) = created else {
		return LOOKUP.skip("make_invoice failed");
	};
	let hash = to_hex(invoice.payment_hash().as_ref());
	let (by_hash, by_invoice) = tokio::join!(
		lookup_invoice(ctx, json!({ "payment_hash": hash })),
		lookup_invoice(ctx, json!({ "invoice": invoice.to_string() })),
	);
	let found = |reply: &Result<Value, String>| match reply {
		Ok(result) if result.get("payment_hash").and_then(Value::as_str) == Some(&hash) => None,
		Ok(_) => Some("returned a different invoice".to_owned()),
		Err(error) => Some(error.clone()),
	};
	match (found(&by_hash), found(&by_invoice)) {
		(None, None) => LOOKUP.pass("found both ways"),
		(Some(error), _) => LOOKUP.fail(format!("by payment_hash: {error}")),
		(_, Some(error)) => LOOKUP.fail(format!("by invoice: {error}")),
	}
}

async fn settle(ctx: &Context) -> Outcome {
	if !ctx.supports("lookup_invoice") {
		return SETTLE.skip("lookup_invoice not advertised");
	}
	let payments = match &ctx.payments {
		Ok(payments) => payments,
		Err(reason) => return SETTLE.skip(reason),
	};
	let params = json!({ "amount": payments.amount_msat, "description": DESCRIPTION });
	let Created { invoice, .. } = match make_invoice(ctx, params).await {
		Ok(created) => created,
		Err(error) => return SETTLE.fail(format!("make_invoice: {error}")),
	};
	let hash = json!({ "payment_hash": to_hex(invoice.payment_hash().as_ref()) });

	match lookup_invoice(ctx, hash.clone()).await {
		Ok(result) if state(&result) == "pending" => {}
		Ok(result) => return SETTLE.fail(format!("new invoice is {}", state(&result))),
		Err(error) => return SETTLE.fail(error),
	}
	if let Err(reason) = payments.reserve() {
		return SETTLE.skip(reason);
	}
	if let Err(error) = payments.node.pay(&invoice.to_string()).await {
		return SETTLE.skip(format!("counterparty couldn't pay it: {error:#}"));
	}

	let mut last = String::new();
	for _ in 0..5 {
		match lookup_invoice(ctx, hash.clone()).await {
			Ok(result) if state(&result) == "settled" => {
				return SETTLE.pass("pending, then settled");
			}
			Ok(result) => last = format!("still {} after payment", state(&result)),
			Err(error) => last = error,
		}
		tokio::time::sleep(Duration::from_secs(1)).await;
	}
	SETTLE.fail(last)
}

async fn unknown(ctx: &Context) -> Outcome {
	if !ctx.supports("lookup_invoice") {
		return UNKNOWN.skip("lookup_invoice not advertised");
	}
	let hash = to_hex(&Keys::generate().secret_key().to_secret_bytes());
	let reply = ctx
		.wallet
		.send(
			Request::new("lookup_invoice", ctx.encryption())
				.params(json!({ "payment_hash": hash })),
		)
		.await;
	match &reply {
		Ok(reply) if reply.error_code() == Some("NOT_FOUND") => UNKNOWN.pass("NOT_FOUND"),
		other => UNKNOWN.fail(format!("expected NOT_FOUND, {}", describe(other))),
	}
}

async fn lookup_invoice(ctx: &Context, params: Value) -> Result<Value, String> {
	let reply =
		ctx.wallet.send(Request::new("lookup_invoice", ctx.encryption()).params(params)).await;
	match &reply {
		Ok(reply) if reply.error().is_none() => {
			reply.result().cloned().ok_or_else(|| "reply has no result".to_owned())
		}
		_ => Err(describe(&reply)),
	}
}

fn state(result: &Value) -> &str {
	match result.get("state").and_then(Value::as_str) {
		Some(state) => state,
		None if result.get("settled_at").is_some_and(|at| !at.is_null()) => "settled",
		None => "pending",
	}
}
