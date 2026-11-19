mod cli;
mod ui;

use std::io::IsTerminal;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use clap::Parser;
use nostr::nips::nip47::NostrWalletConnectUri;
use nwc_check::check::{Outcome, Summary};
use nwc_check::checks::{self, Context};
use nwc_check::report::Report;
use nwc_check::wallet::Wallet;

use crate::cli::{Cli, Command, Format, RunArgs};
use crate::ui::Ui;

#[tokio::main]
async fn main() -> ExitCode {
	match Cli::parse().command {
		Command::List => {
			ui::write_stdout(&ui::catalog());
			ExitCode::SUCCESS
		}
		Command::Run(args) => match run(args).await {
			Ok(code) => code,
			Err(error) => {
				eprintln!("{} {error:#}", console::style("error:").red().bold());
				ExitCode::from(2)
			}
		},
	}
}

async fn run(args: RunArgs) -> Result<ExitCode> {
	let started = Instant::now();
	let uri = NostrWalletConnectUri::parse(args.uri.trim())
		.context("invalid connection string, expected nostr+walletconnect://...")?;
	let wallet_key = uri.public_key.to_hex();
	let relays: Vec<String> = uri.relays.iter().map(ToString::to_string).collect();

	let text = matches!(args.format, Format::Text);
	let no_color = args.no_color || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
	let terminal = std::io::stdout().is_terminal();
	let ui = Ui::new(text && terminal && !no_color, text && terminal);
	let say = |f: &dyn Fn(&Ui)| {
		if text {
			f(&ui)
		}
	};

	say(&|ui| ui.header(&wallet_key, &relays));

	ui.busy("Connecting to relays");
	let wallet = Wallet::connect(uri, Duration::from_secs(args.timeout))
		.await
		.context("failed to connect to the wallet's relays")?;
	say(&|ui| ui.step(true, format!("Connected to {}", plural(relays.len(), "relay"))));

	ui.busy("Reading wallet info");
	let ctx = Context::load(wallet, args.network.map(|network| network.as_str().to_owned()))
		.await
		.context("failed to query the wallet service")?;
	say(&|ui| match &ctx.info {
		Some(_) => ui.step(
			true,
			format!(
				"Found wallet info: {}, encryption {}",
				plural(ctx.methods.len(), "method"),
				if ctx.schemes.is_empty() { "nip04".to_owned() } else { ctx.schemes.join(" + ") }
			),
		),
		None => ui.step(false, "No wallet info event found"),
	});
	if ctx.get_info.is_err() {
		say(&|ui| {
			ui.step(
				false,
				"The wallet didn't answer get_info. Check that the wallet service is online and the connection string is current.",
			)
		});
	}

	let total = checks::catalog().len();
	let mut results: Vec<Outcome> = Vec::new();
	ui.busy(format!("Running checks 0/{total}"));
	checks::run_by_section(&ctx, |section, outcomes| {
		let shown: Vec<Outcome> =
			outcomes.into_iter().filter(|outcome| selected(&args.only, outcome.id)).collect();
		say(&|ui| ui.section(section, &shown));
		results.extend(shown);
		ui.busy(format!("Running checks {}/{total}", results.len()));
	})
	.await;
	ctx.wallet.disconnect().await;
	ui.finish();

	let summary = Summary::of(&results);
	if text {
		ui.problems(&results);
		ui.summary(&summary, started.elapsed(), args.strict);
	} else {
		let report = Report::new(wallet_key, relays, started.elapsed().as_millis(), results);
		ui::write_stdout(&format!("{}\n", report.to_json()));
	}

	Ok(exit_code(&summary, args.strict))
}

fn selected(only: &[String], id: &str) -> bool {
	only.is_empty() || only.iter().any(|prefix| id.starts_with(&prefix.to_uppercase()))
}

fn plural(count: usize, noun: &str) -> String {
	if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}

fn exit_code(summary: &Summary, strict: bool) -> ExitCode {
	if summary.failed > 0 || (strict && summary.warnings > 0) {
		ExitCode::FAILURE
	} else {
		ExitCode::SUCCESS
	}
}
