use nostr_sdk::prelude::Timestamp;
use serde_json::Value;

use super::{Context, describe};
use crate::check::{Check, Level, Outcome, Spec};
use crate::wallet::Scheme;

const INFO_EVENT: Check = Check::new(
	"DISC-01",
	Level::Must,
	Spec::Nip47,
	"Info event is published",
	"Apps learn what the wallet supports from this event.",
);
const METHODS: Check = Check::new(
	"DISC-02",
	Level::Must,
	Spec::Nip47,
	"Info event lists methods",
	"Without it, apps can't tell which requests the wallet accepts.",
);
const NIP44: Check = Check::new(
	"DISC-03",
	Level::Should,
	Spec::Nip47,
	"NIP-44 encryption is advertised",
	"NIP-04 is deprecated; NIP-44 is the encryption clients should use.",
);
const GET_INFO_METHODS: Check = Check::new(
	"DISC-04",
	Level::Should,
	Spec::Nip47,
	"get_info methods match the info event",
	"Apps may call methods the wallet never advertised.",
);
const NETWORK: Check = Check::new(
	"DISC-05",
	Level::Must,
	Spec::Nip47,
	"get_info reports a valid network",
	"Apps check the network so they never pay on the wrong chain.",
);
const EXTENSIONS: Check = Check::new(
	"DISC-06",
	Level::Should,
	Spec::Nip47,
	"Extensions match between info event and get_info",
	"Apps rely on extensions being advertised the same way in both places.",
);

pub const CHECKS: &[&Check] =
	&[&INFO_EVENT, &METHODS, &NIP44, &GET_INFO_METHODS, &NETWORK, &EXTENSIONS];

const NETWORKS: [&str; 4] = ["mainnet", "testnet", "signet", "regtest"];

pub async fn run(ctx: &Context) -> Vec<Outcome> {
	let Some(info) = &ctx.info else {
		return vec![
			INFO_EVENT.fail("no kind 13194 event found on the connection's relays"),
			METHODS.skip("no info event"),
			NIP44.skip("no info event"),
			GET_INFO_METHODS.skip("no info event"),
			network(ctx).citing(&ctx.shared("get_info")),
			EXTENSIONS.skip("no info event"),
		];
	};

	let info_extensions: Vec<&str> = info
		.tags
		.iter()
		.find(|tag| tag.kind() == "extensions")
		.and_then(|tag| tag.content())
		.map(|value| value.split_whitespace().collect())
		.unwrap_or_default();

	vec![
		INFO_EVENT.pass(format!("published {}", age(info.created_at))),
		METHODS.verdict(!ctx.methods.is_empty(), list_or("methods", &ctx.methods)),
		NIP44.verdict(ctx.advertises(Scheme::Nip44), list_or("encryption", &ctx.schemes)),
		get_info_methods(ctx).citing(&ctx.shared("get_info")),
		network(ctx).citing(&ctx.shared("get_info")),
		extensions(ctx, &info_extensions).citing(&ctx.shared("get_info")),
	]
}

fn get_info_methods(ctx: &Context) -> Outcome {
	let Some(result) = ctx.get_info.as_ref().ok().and_then(|reply| reply.result()) else {
		return GET_INFO_METHODS.skip(format!("get_info: {}", describe(&ctx.get_info)));
	};
	let methods = strings(result.get("methods"));
	let unadvertised: Vec<&str> =
		methods.iter().copied().filter(|method| !ctx.supports(method)).collect();
	if unadvertised.is_empty() {
		GET_INFO_METHODS.pass(format!("{} methods, all advertised", methods.len()))
	} else {
		GET_INFO_METHODS.fail(format!("not in the info event: {}", unadvertised.join(", ")))
	}
}

fn network(ctx: &Context) -> Outcome {
	let Some(result) = ctx.get_info.as_ref().ok().and_then(|reply| reply.result()) else {
		return NETWORK.fail(format!("get_info: {}", describe(&ctx.get_info)));
	};
	let network = result.get("network").and_then(Value::as_str);
	match (network, ctx.expected_network.as_deref()) {
		(Some(network), Some(expected)) if network != expected => {
			NETWORK.fail(format!("reported {network}, expected {expected}"))
		}
		(Some(network), _) if NETWORKS.contains(&network) => NETWORK.pass(network),
		(Some(network), _) => NETWORK.fail(format!("unknown network {network:?}")),
		(None, _) => NETWORK.fail("get_info result has no network"),
	}
}

fn extensions(ctx: &Context, info_extensions: &[&str]) -> Outcome {
	let reported = ctx
		.get_info
		.as_ref()
		.ok()
		.and_then(|reply| reply.result())
		.map(|result| strings(result.get("extensions")))
		.unwrap_or_default();
	if info_extensions.is_empty() && reported.is_empty() {
		return EXTENSIONS.skip("no extensions advertised");
	}
	let missing: Vec<&str> =
		reported.iter().copied().filter(|ext| !info_extensions.contains(ext)).collect();
	if missing.is_empty() {
		EXTENSIONS.pass(list_or("extensions", info_extensions))
	} else {
		EXTENSIONS.fail(format!("in get_info but not in the info event: {}", missing.join(", ")))
	}
}

fn strings(value: Option<&Value>) -> Vec<&str> {
	value
		.and_then(Value::as_array)
		.map(|items| items.iter().filter_map(Value::as_str).collect())
		.unwrap_or_default()
}

fn list_or(label: &str, items: &[impl AsRef<str>]) -> String {
	if items.is_empty() {
		format!("no {label}")
	} else {
		items.iter().map(AsRef::as_ref).collect::<Vec<_>>().join(", ")
	}
}

fn age(created_at: Timestamp) -> String {
	let secs = Timestamp::now().as_secs().saturating_sub(created_at.as_secs());
	match secs {
		0..60 => "just now".to_owned(),
		60..3_600 => format!("{} min ago", secs / 60),
		3_600..86_400 => format!("{} h ago", secs / 3_600),
		_ => format!("{} days ago", secs / 86_400),
	}
}
