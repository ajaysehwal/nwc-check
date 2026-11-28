use std::time::Duration;

use super::{Context, describe, tracked};
use crate::check::{Check, Level, Outcome, Severity, Spec};
use crate::wallet::{Request, WalletError};

/// Small enough that nwc-check never becomes the attack.
pub const BURST: usize = 60;
const RECOVERY: Duration = Duration::from_secs(5);

const RATE_LIMIT: Check = Check::new(
	"SAFE-15",
	Level::Should,
	Severity::Medium,
	Spec::Safety,
	"A burst of requests is rate limited and the wallet recovers",
	"A leaked connection or a buggy app can flood the wallet with requests that cost it storage, CPU or fees.",
);

pub const CHECKS: &[&Check] = &[&RATE_LIMIT];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	if !ctx.flood {
		return vec![
			RATE_LIMIT.skip("off by default, since it can trip alerts (see --with-flood)"),
		];
	}
	vec![tracked(flood(ctx)).await]
}

async fn flood(ctx: &Context) -> Outcome {
	let requests = (0..BURST).map(|_| Request::new("get_info", ctx.encryption())).collect();
	let replies = ctx.wallet.burst(requests).await;

	let (mut limited, mut answered, mut silent, mut rejected) = (0, 0, 0, 0);
	for reply in &replies {
		match reply {
			Ok(reply) if reply.error_code() == Some("RATE_LIMITED") => limited += 1,
			Ok(_) => answered += 1,
			Err(WalletError::Rejected(_) | WalletError::Relay(_)) => rejected += 1,
			Err(_) => silent += 1,
		}
	}
	if rejected > 0 {
		return RATE_LIMIT.skip(format!(
			"the relays turned away {rejected} of {BURST} requests, so the wallet never saw the burst"
		));
	}

	tokio::time::sleep(RECOVERY).await;
	let after = ctx.wallet.send(Request::new("get_info", ctx.encryption())).await;
	if let Err(error) = &after {
		return RATE_LIMIT.fail(format!(
			"stopped answering after {BURST} requests: get_info {}s later got {error}",
			RECOVERY.as_secs()
		));
	}

	let tally = format!("{limited} RATE_LIMITED, {answered} answered, {silent} unanswered");
	if limited > 0 {
		RATE_LIMIT.pass(format!("{tally}; then get_info {}", describe(&after)))
	} else if silent > 0 {
		RATE_LIMIT.fail(format!("{tally}: requests were dropped without RATE_LIMITED"))
	} else {
		RATE_LIMIT.fail(format!("all {BURST} requests were answered: {tally}"))
	}
}
