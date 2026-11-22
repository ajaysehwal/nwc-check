use serde_json::Value;

use super::{Context, describe};
use crate::check::{Check, Level, Outcome, Spec};
use crate::wallet::{Reply, Request, WalletError};

const RESULT_TYPE: Check = Check::new(
	"RESP-01",
	Level::Must,
	Spec::Nip47,
	"result_type matches the method",
	"Clients match a reply to its method by result_type.",
);
const NULL_ERROR: Check = Check::new(
	"RESP-02",
	Level::Must,
	Spec::Nip47,
	"Successful reply has a null error",
	"A non-null error makes clients treat a success as a failure.",
);
const ERROR_SHAPE: Check = Check::new(
	"RESP-03",
	Level::Must,
	Spec::Nip47,
	"Error has a code and a message",
	"Clients show the message to users and act on the code.",
);
const TAGS: Check = Check::new(
	"RESP-04",
	Level::Should,
	Spec::Nip47,
	"Reply references the request and client",
	"Without e and p tags, clients can't match replies to their requests.",
);
const BALANCE: Check = Check::new(
	"RESP-05",
	Level::Must,
	Spec::Nip47,
	"get_balance returns a balance in msats",
	"Apps show this balance to users.",
);

pub const CHECKS: &[&Check] = &[&RESULT_TYPE, &NULL_ERROR, &ERROR_SHAPE, &TAGS, &BALANCE];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	vec![result_type(ctx), null_error(ctx), error_shape(ctx), tags(ctx), balance(ctx).await]
}

fn result_type(ctx: &Context) -> Outcome {
	match &ctx.get_info {
		Ok(reply) => match reply.result_type() {
			Some("get_info") => RESULT_TYPE.pass("get_info"),
			other => RESULT_TYPE.fail(format!("expected get_info, got {other:?}")),
		},
		Err(error) => RESULT_TYPE.skip(format!("get_info: {error}")),
	}
}

fn null_error(ctx: &Context) -> Outcome {
	match &ctx.get_info {
		Ok(reply) if reply.result().is_some() => match reply.body.get("error") {
			None | Some(Value::Null) => NULL_ERROR.pass("error is null"),
			Some(error) => NULL_ERROR.fail(format!("error is {error}")),
		},
		other => NULL_ERROR.skip(format!("get_info: {}", describe(other))),
	}
}

fn error_shape(ctx: &Context) -> Outcome {
	let Ok(reply) = &ctx.unknown_method else {
		return ERROR_SHAPE.skip(format!("unknown method: {}", describe(&ctx.unknown_method)));
	};
	let Some(error) = reply.error() else {
		return ERROR_SHAPE.skip("unknown method did not return an error");
	};
	let code = error.get("code").and_then(Value::as_str);
	let message = error.get("message").and_then(Value::as_str);
	match (code, message) {
		(Some(code), Some(_)) => ERROR_SHAPE.pass(code),
		_ => ERROR_SHAPE.fail(format!("error is {error}")),
	}
}

fn tags(ctx: &Context) -> Outcome {
	match &ctx.get_info {
		Ok(reply) => {
			let problems = tag_problems(reply);
			TAGS.verdict(
				problems.is_empty(),
				if problems.is_empty() {
					"e and p tags present".to_owned()
				} else {
					problems.join(", ")
				},
			)
		}
		Err(WalletError::NoETag) => TAGS.fail("reply has no e tag"),
		Err(error) => TAGS.skip(format!("get_info: {error}")),
	}
}

fn tag_problems(reply: &Reply) -> Vec<&'static str> {
	let mut problems = Vec::new();
	if !reply.event.tags.event_ids().any(|id| id == reply.request_id) {
		problems.push("no e tag for the request");
	}
	if !reply.event.tags.public_keys().any(|pk| pk == reply.client) {
		problems.push("no p tag for the client");
	}
	problems
}

async fn balance(ctx: &Context) -> Outcome {
	if !ctx.supports("get_balance") {
		return BALANCE.skip("get_balance not advertised");
	}
	match ctx.wallet.send(Request::new("get_balance", ctx.encryption())).await {
		Ok(reply) => match reply.result().and_then(|result| result.get("balance")) {
			Some(balance) if balance.is_u64() => BALANCE.pass(format!("{balance} msats")),
			Some(balance) => BALANCE.fail(format!("balance is {balance}")),
			None => BALANCE.fail(describe(&Ok(reply))),
		},
		Err(error) => BALANCE.fail(error.to_string()),
	}
}
