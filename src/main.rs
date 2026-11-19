mod cli;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context as _, Result};
use clap::Parser;
use nostr::nips::nip47::NostrWalletConnectUri;
use nwc_check::check::Summary;
use nwc_check::checks::{self, Context};
use nwc_check::report::{self, Report};
use nwc_check::wallet::Wallet;

use crate::cli::{Cli, Command, Format, RunArgs};

#[tokio::main]
async fn main() -> ExitCode {
	match Cli::parse().command {
		Command::List => {
			print!("{}", report::catalog(&checks::catalog()));
			ExitCode::SUCCESS
		}
		Command::Run(args) => match run(args).await {
			Ok(code) => code,
			Err(error) => {
				eprintln!("error: {error:#}");
				ExitCode::from(2)
			}
		},
	}
}

async fn run(args: RunArgs) -> Result<ExitCode> {
	let uri = NostrWalletConnectUri::parse(args.uri.trim())
		.context("invalid connection string, expected nostr+walletconnect://...")?;
	let wallet = Wallet::connect(uri, Duration::from_secs(args.timeout))
		.await
		.context("failed to connect to the wallet's relays")?;
	let (wallet_key, relays) = (wallet.public_key().to_hex(), wallet.relays());

	let ctx = Context::load(wallet, args.network.map(|network| network.as_str().to_owned()))
		.await
		.context("failed to query the wallet service")?;
	let mut results = checks::run(&ctx).await;
	ctx.wallet.disconnect().await;

	if !args.only.is_empty() {
		results.retain(|result| {
			args.only.iter().any(|prefix| result.id.starts_with(&prefix.to_uppercase()))
		});
	}

	let report = Report::new(wallet_key, relays, results);
	match args.format {
		Format::Text => {
			let no_color =
				args.no_color || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
			let color = !no_color && std::io::stdout().is_terminal();
			print!("{}", report::text(&report, color));
		}
		Format::Json => println!("{}", report::json(&report)),
	}

	Ok(exit_code(&report.summary, args.strict))
}

fn exit_code(summary: &Summary, strict: bool) -> ExitCode {
	if summary.failed > 0 || (strict && summary.warnings > 0) {
		ExitCode::FAILURE
	} else {
		ExitCode::SUCCESS
	}
}
