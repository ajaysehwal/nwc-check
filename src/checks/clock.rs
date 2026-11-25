use super::{Context, describe, tracked};
use crate::check::{Check, Level, Outcome, Spec};
use crate::wallet::{Request, WalletError};
use nostr_sdk::prelude::Timestamp;

const FUTURE: Check = Check::new(
	"SAFE-07",
	Level::Safety,
	Spec::Safety,
	"A future-dated request doesn't lock out later ones",
	"A wallet that remembers the newest request time to refuse replays can lock out a client whose clock runs ahead, unless it caps that time at now.",
);

pub const CHECKS: &[&Check] = &[&FUTURE];

/// How far ahead the request is dated. Relays refuse events dated too far ahead (15 minutes is a
/// common limit), so this stays under it while still being past a ten minute replay window.
const AHEAD_SECONDS: u64 = 14 * 60;

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	vec![tracked(future(ctx)).await]
}

async fn future(ctx: &Context) -> Outcome {
	// The wallet has to answer a normal request first, or a later silence proves nothing.
	match ctx.wallet.send(Request::new("get_info", ctx.encryption())).await {
		Ok(_) => {}
		Err(WalletError::NoReply) => return FUTURE.skip("get_info isn't answered"),
		Err(error) => return FUTURE.skip(format!("get_info: {error}")),
	}

	let ahead = Timestamp::from_secs(Timestamp::now().as_secs() + AHEAD_SECONDS);
	// Whether the wallet answers it or ignores it doesn't matter, only what comes next.
	let dated = ctx.wallet.send(Request::new("get_info", ctx.encryption()).created_at(ahead)).await;
	if let Err(WalletError::Rejected(_)) = dated {
		return FUTURE.skip("relays dropped the future-dated request");
	}

	let next = ctx.wallet.send(Request::new("get_info", ctx.encryption())).await;
	match next {
		Ok(_) => FUTURE.pass("the next request was still answered"),
		Err(WalletError::NoReply) => FUTURE.fail(format!(
			"a request dated {} minutes ahead blocked the next one",
			AHEAD_SECONDS / 60
		)),
		other => FUTURE.skip(format!("the next request: {}", describe(&other))),
	}
}
