use serde_json::Value;

use super::{Context, describe};
use crate::check::{Check, Level, Outcome};
use crate::wallet::Scheme;

const INFO_EVENT: Check = Check::new("DISC-01", Level::Must, "Info event is published");
const METHODS: Check = Check::new("DISC-02", Level::Must, "Info event lists methods");
const NIP44: Check = Check::new("DISC-03", Level::Should, "NIP-44 encryption is advertised");
const GET_INFO_METHODS: Check =
	Check::new("DISC-04", Level::Should, "get_info methods match the info event");
const NETWORK: Check = Check::new("DISC-05", Level::Must, "get_info reports a valid network");
const EXTENSIONS: Check =
	Check::new("DISC-06", Level::Should, "Extensions match between info event and get_info");

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
			network(ctx),
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
		INFO_EVENT.pass(format!("published at {}", info.created_at)),
		METHODS.verdict(!ctx.methods.is_empty(), list_or("methods", &ctx.methods)),
		NIP44.verdict(ctx.advertises(Scheme::Nip44), list_or("encryption", &ctx.schemes)),
		get_info_methods(ctx),
		network(ctx),
		extensions(ctx, &info_extensions),
	]
}

fn get_info_methods(ctx: &Context) -> Outcome {
	let Some(result) = ctx.get_info.as_ref().ok().and_then(|reply| reply.result()) else {
		return GET_INFO_METHODS.skip(format!("get_info {}", describe(&ctx.get_info)));
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
		return NETWORK.fail(format!("get_info {}", describe(&ctx.get_info)));
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
