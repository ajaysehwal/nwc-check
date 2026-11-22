use nostr_sdk::prelude::Keys;
use serde_json::json;

use super::{Context, UNKNOWN_METHOD, describe};
use crate::check::{Check, Level, Outcome, Spec};
use crate::evidence::track;
use crate::wallet::{Reply, Request, WalletError};

const NOT_IMPLEMENTED: Check = Check::new(
	"ERR-01",
	Level::Should,
	Spec::Nip47,
	"Unknown method returns NOT_IMPLEMENTED",
	"Clients need NOT_IMPLEMENTED to know a feature is missing.",
);
const UNAUTHORIZED: Check = Check::new(
	"ERR-02",
	Level::Should,
	Spec::Nip47,
	"Unknown client returns UNAUTHORIZED",
	"Requests from keys that aren't connected must be clearly refused.",
);
const MALFORMED: Check = Check::new(
	"ERR-03",
	Level::Safety,
	Spec::Nip47,
	"Malformed params return an error",
	"Ignoring bad input hides bugs and leaves clients waiting.",
);

pub const CHECKS: &[&Check] = &[&NOT_IMPLEMENTED, &UNAUTHORIZED, &MALFORMED];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let stranger = Request::new("get_info", ctx.encryption()).signer(Keys::generate());
	let malformed = Request::new("lookup_invoice", ctx.encryption()).params(json!({}));
	let ((stranger, stranger_evidence), (malformed, malformed_evidence)) = tokio::join!(
		track(ctx.wallet.send(stranger)),
		track(async {
			if ctx.supports("lookup_invoice") {
				Some(ctx.wallet.send(malformed).await)
			} else {
				None
			}
		})
	);

	vec![
		expect_code(&NOT_IMPLEMENTED, "NOT_IMPLEMENTED", &ctx.unknown_method)
			.citing(&ctx.shared(UNKNOWN_METHOD)),
		expect_code(&UNAUTHORIZED, "UNAUTHORIZED", &stranger).with(&stranger_evidence),
		match malformed {
			None => MALFORMED.skip("lookup_invoice not advertised"),
			Some(Ok(reply)) if reply.error().is_some() => MALFORMED.pass(describe(&Ok(reply))),
			Some(other) => MALFORMED.fail(describe(&other)),
		}
		.with(&malformed_evidence),
	]
}

fn expect_code(check: &Check, code: &str, reply: &Result<Reply, WalletError>) -> Outcome {
	match reply {
		Ok(reply) if reply.error_code() == Some(code) => check.pass(code),
		other => check.fail(format!("expected {code}, {}", describe(other))),
	}
}
