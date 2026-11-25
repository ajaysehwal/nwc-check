use super::{Context, describe};
use crate::check::{Check, Level, Outcome, Severity, Spec};
use crate::evidence::track;
use crate::wallet::{Encryption, Reply, Request, Scheme, WalletError};

const NIP44_REPLY: Check = Check::new(
	"ENC-01",
	Level::Must,
	Severity::Medium,
	Spec::Nip47,
	"NIP-44 request gets a NIP-44 reply",
	"A client can't read a reply encrypted differently from its request.",
);
const NIP04_REPLY: Check = Check::new(
	"ENC-02",
	Level::Must,
	Severity::Medium,
	Spec::Nip47,
	"NIP-04 request gets a NIP-04 reply",
	"A client can't read a reply encrypted differently from its request.",
);
const UNSUPPORTED: Check = Check::new(
	"ENC-03",
	Level::Must,
	Severity::Low,
	Spec::Nip47,
	"Unsupported scheme returns UNSUPPORTED_ENCRYPTION",
	"Clients need this error to fall back to a scheme the wallet supports.",
);
const UNTAGGED: Check = Check::new(
	"ENC-04",
	Level::Must,
	Severity::Medium,
	Spec::Nip47,
	"Untagged request is treated as NIP-04",
	"Older clients send no encryption tag and expect a NIP-04 reply.",
);

pub const CHECKS: &[&Check] = &[&NIP44_REPLY, &NIP04_REPLY, &UNSUPPORTED, &UNTAGGED];

const UNSUPPORTED_SCHEME: &str = "nwc_check_unsupported";

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let get_info = |encryption| Request::new("get_info", encryption);
	let (
		(nip44, nip44_evidence),
		(nip04, nip04_evidence),
		(unsupported, unsupported_evidence),
		(untagged, untagged_evidence),
	) = tokio::join!(
		track(ctx.wallet.send(get_info(Encryption::Tagged(Scheme::Nip44)))),
		track(ctx.wallet.send(get_info(Encryption::Tagged(Scheme::Nip04)))),
		track(ctx.wallet.send(get_info(Encryption::Unsupported(UNSUPPORTED_SCHEME)))),
		track(ctx.wallet.send(get_info(Encryption::Untagged))),
	);

	let roundtrip = |check: &Check, scheme: Scheme, reply: &Result<Reply, WalletError>| {
		if !ctx.advertises(scheme) {
			return check.skip(format!("{} not advertised", scheme.as_str()));
		}
		match reply {
			Ok(reply) if reply.scheme == scheme => {
				check.pass(format!("answered with {}", scheme.as_str()))
			}
			Ok(reply) => check.fail(format!("replied with {}", reply.scheme.as_str())),
			Err(error) => check.fail(error.to_string()),
		}
	};

	let unsupported = match &unsupported {
		Ok(reply) if reply.error_code() == Some("UNSUPPORTED_ENCRYPTION") => {
			UNSUPPORTED.pass("UNSUPPORTED_ENCRYPTION")
		}
		other => UNSUPPORTED.fail(describe(other)),
	};

	let untagged = match &untagged {
		Ok(reply) if reply.scheme == Scheme::Nip04 && reply.error().is_none() => {
			UNTAGGED.pass("answered with NIP-04")
		}
		Ok(reply) if reply.error_code() == Some("UNSUPPORTED_ENCRYPTION") => {
			UNTAGGED.pass("NIP-04 not supported, returned UNSUPPORTED_ENCRYPTION")
		}
		Ok(reply) => {
			UNTAGGED.fail(format!("{} using {}", describe(&untagged), reply.scheme.as_str()))
		}
		Err(error) => UNTAGGED.fail(error.to_string()),
	};

	vec![
		roundtrip(&NIP44_REPLY, Scheme::Nip44, &nip44).with(&nip44_evidence),
		roundtrip(&NIP04_REPLY, Scheme::Nip04, &nip04).with(&nip04_evidence),
		unsupported.with(&unsupported_evidence),
		untagged.with(&untagged_evidence),
	]
}
