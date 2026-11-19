use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use nwc::prelude::*;

/// Tests a Nostr Wallet Connect (NIP-47) wallet service.
#[derive(Parser)]
#[command(name = "nwc-check", version)]
struct Cli {
	#[command(subcommand)]
	command: Command,
}

#[derive(Subcommand)]
enum Command {
	/// Run the read-only checks against a wallet service.
	Run {
		/// The connection string: nostr+walletconnect://...
		uri: String,
		/// How long to wait for each answer, in seconds.
		#[arg(long, default_value_t = 10)]
		timeout: u64,
	},
}

/// The outcome of one check.
struct CheckResult {
	id: &'static str,
	passed: bool,
	message: String,
}

impl CheckResult {
	fn pass(id: &'static str, message: impl Into<String>) -> Self {
		Self { id, passed: true, message: message.into() }
	}

	fn fail(id: &'static str, message: impl Into<String>) -> Self {
		Self { id, passed: false, message: message.into() }
	}
}

#[tokio::main]
async fn main() -> Result<()> {
	match Cli::parse().command {
		Command::Run { uri, timeout } => run(&uri, Duration::from_secs(timeout)).await,
	}
}

async fn run(uri: &str, timeout: Duration) -> Result<()> {
	let uri = NostrWalletConnectUri::parse(uri).context("invalid connection string")?;
	println!("Wallet service: {}", uri.public_key.to_hex());

	let mut results = check_info_event(&uri, timeout).await?;
	results.push(check_get_info(&uri, timeout).await);

	let passed = results.iter().filter(|r| r.passed).count();
	for result in &results {
		let mark = if result.passed { "PASS" } else { "FAIL" };
		println!("[{mark}] {:<8} {}", result.id, result.message);
	}
	println!("{passed}/{} checks passed", results.len());

	if passed < results.len() {
		std::process::exit(1);
	}
	Ok(())
}

/// DISC-01..03: the wallet service publishes an info event listing its methods and encryption.
async fn check_info_event(
	uri: &NostrWalletConnectUri,
	timeout: Duration,
) -> Result<Vec<CheckResult>> {
	let client = Client::default();
	for relay in &uri.relays {
		client.add_relay(relay.as_str()).await?;
	}
	client.connect().and_wait(timeout).await;

	let filter = Filter::new().kind(Kind::WalletConnectInfo).author(uri.public_key);
	let events = client.fetch_events(filter).timeout(timeout).await?;
	client.shutdown().await;

	let Some(info) = events.into_iter().max_by_key(|event| event.created_at) else {
		return Ok(vec![CheckResult::fail("DISC-01", "no info event (kind 13194) found")]);
	};

	let mut results = vec![CheckResult::pass("DISC-01", "info event found")];

	let methods: Vec<&str> = info.content.split_whitespace().collect();
	results.push(if methods.is_empty() {
		CheckResult::fail("DISC-02", "info event lists no methods")
	} else {
		CheckResult::pass("DISC-02", format!("methods: {}", methods.join(", ")))
	});

	let encryption = info
		.tags
		.iter()
		.find(|tag| tag.kind() == "encryption")
		.and_then(|tag| tag.content())
		.map(str::to_owned);
	results.push(match encryption {
		Some(schemes) if schemes.split_whitespace().any(|s| s == "nip44_v2") => {
			CheckResult::pass("DISC-03", format!("encryption: {schemes}"))
		}
		Some(schemes) => CheckResult::fail("DISC-03", format!("no nip44_v2 in: {schemes}")),
		None => CheckResult::fail("DISC-03", "no encryption tag (implies NIP-04 only)"),
	});

	Ok(results)
}

/// CORE-INFO: `get_info` answers with a valid network.
async fn check_get_info(uri: &NostrWalletConnectUri, timeout: Duration) -> CheckResult {
	let nwc = NostrWalletConnect::new(uri.clone());
	let response = nwc.get_info().timeout(timeout).await;
	nwc.shutdown().await;

	match response {
		Ok(info) => match info.network.as_deref() {
			Some("mainnet" | "testnet" | "signet" | "regtest") => CheckResult::pass(
				"INFO-01",
				format!("get_info: network {}", info.network.unwrap_or_default()),
			),
			other => CheckResult::fail("INFO-01", format!("get_info: invalid network {other:?}")),
		},
		Err(e) => CheckResult::fail("INFO-01", format!("get_info failed: {e}")),
	}
}
