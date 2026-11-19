use nostr_sdk::prelude::Timestamp;

use super::{Context, describe};
use crate::check::{Check, Level, Outcome};
use crate::wallet::{Request, WalletError};

const EXPIRED: Check = Check::new("SAFE-01", Level::Safety, "Expired requests are ignored");

pub const CHECKS: &[&Check] = &[&EXPIRED];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let expired = Timestamp::from_secs(Timestamp::now().as_secs() - 60);
	let request = Request::new("get_info", ctx.encryption()).expiration(expired);
	let outcome = match ctx.wallet.send(request).await {
		Err(WalletError::NoReply) => EXPIRED.pass("no reply"),
		Err(WalletError::Rejected(_)) => EXPIRED.skip("relays dropped the expired request"),
		Ok(reply) if reply.error().is_some() => EXPIRED.pass(describe(&Ok(reply))),
		other => EXPIRED.fail(format!("expired request was answered: {}", describe(&other))),
	};
	vec![outcome]
}
