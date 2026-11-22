mod cli;
mod config;
mod polar;
mod ui;

use std::io::IsTerminal;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use clap::Parser;
use nostr::nips::nip47::NostrWalletConnectUri;
use nwc_check::check::{Outcome, Summary};
use nwc_check::checks::{self, Context, Payments};
use nwc_check::counterparty::{Counterparty, Lnd};
use nwc_check::report::Report;
use nwc_check::wallet::{Wallet, WalletError};

use crate::cli::{Cli, Command, Format, InitArgs, RunArgs};
use crate::config::{Config, LndSettings, Settings};
use crate::ui::Ui;

#[tokio::main]
async fn main() -> ExitCode {
	let result = match Cli::parse().command {
		Command::List => {
			ui::write_stdout(&ui::catalog());
			Ok(ExitCode::SUCCESS)
		}
		Command::Init(args) => init(args),
		Command::Run(args) => run(*args).await,
	};
	result.unwrap_or_else(|error| {
		eprintln!("{} {error:#}", console::style("error:").red().bold());
		ExitCode::from(2)
	})
}

async fn run(args: RunArgs) -> Result<ExitCode> {
	let started = Instant::now();
	let uri = NostrWalletConnectUri::parse(args.uri.trim())
		.context("invalid connection string, expected nostr+walletconnect://...")?;
	let loaded = Config::load(args.config.as_deref())?;
	let source = loaded.as_ref().map(|(path, _)| path.display().to_string());
	let settings = Settings::resolve(&args, loaded.map(|(_, config)| config).unwrap_or_default())?;
	let wallet_key = uri.public_key.to_hex();
	let relays: Vec<String> = uri.relays.iter().map(ToString::to_string).collect();

	let text = matches!(args.format, Format::Text | Format::Html);
	let no_color = args.no_color || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
	let terminal = std::io::stdout().is_terminal();
	let ui = Ui::new(text && terminal && !no_color, text && terminal);
	let say = |f: &dyn Fn(&Ui)| {
		if text {
			f(&ui)
		}
	};

	say(&|ui| ui.header(&wallet_key, &relays));
	if let Some(source) = &source {
		say(&|ui| ui.step(true, format!("Using settings from {source}")));
	}

	ui.busy("Connecting to relays");
	let wallet = Wallet::connect(uri, settings.timeout)
		.await
		.context("failed to connect to the wallet's relays")?;
	say(&|ui| ui.step(true, format!("Connected to {}", plural(relays.len(), "relay"))));
	let readonly = match &args.readonly_uri {
		Some(readonly) => {
			let readonly = NostrWalletConnectUri::parse(readonly.trim())
				.context("invalid read-only connection string")?;
			let readonly = Wallet::connect(readonly, settings.timeout)
				.await
				.context("failed to connect to the read-only connection's relays")?;
			say(&|ui| ui.step(true, "Connected the read-only connection"));
			Some(readonly)
		}
		None => None,
	};

	ui.busy("Reading wallet info");
	let mut ctx =
		Context::load(wallet, settings.network.map(|network| network.as_str().to_owned()))
			.await
			.context("failed to query the wallet service")?;
	if let Some(readonly) = readonly {
		ctx = ctx.with_readonly(readonly);
	}
	if let Some(lnd) = &settings.lnd {
		ctx = if settings.allow_spend {
			ui.busy("Connecting to the counterparty node");
			ctx.with_payments(counterparty(lnd, &settings).await?)
		} else {
			ctx.block_payments("spending is off (see --allow-spend)")
		};
		say(&|ui| match &ctx.payments {
			Ok(payments) => ui.step(
				true,
				format!(
					"Counterparty LND on {}, up to {} sats",
					payments.network, settings.max_sats
				),
			),
			Err(reason) => ui.step(false, format!("Payment checks off: {reason}")),
		});
	}
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
	match &ctx.get_info {
		Err(WalletError::NoETag) => say(&|ui| {
			ui.step(
				false,
				"The wallet's replies have no e tag, so most checks can't match them to their requests.",
			)
		}),
		Err(_) => say(&|ui| {
			ui.step(
				false,
				"The wallet didn't answer get_info. Check that the wallet service is online and the connection string is current.",
			)
		}),
		Ok(_) => {}
	}

	let total = checks::catalog().len();
	let mut results: Vec<Outcome> = Vec::new();
	ui.busy(format!("Running checks 0/{total}"));
	checks::run_by_section(&ctx, |section, outcomes| {
		let shown: Vec<Outcome> =
			outcomes.into_iter().filter(|outcome| selected(&settings.only, outcome.id)).collect();
		say(&|ui| ui.section(section, &shown));
		results.extend(shown);
		ui.busy(format!("Running checks {}/{total}", results.len()));
	})
	.await;
	ctx.disconnect().await;
	ui.finish();

	let summary = Summary::of(&results);
	let elapsed = started.elapsed();
	let report = || {
		Report::new(
			wallet_key.clone(),
			relays.clone(),
			elapsed.as_millis(),
			settings.strict,
			results.clone(),
		)
	};
	if text {
		ui.problems(&results);
		ui.summary(&summary, elapsed, settings.strict);
	}
	match args.format {
		Format::Text => {}
		Format::Json => write_report(args.output.as_deref(), &format!("{}\n", report().to_json()))?,
		Format::Html => {
			let path = args.output.as_deref().unwrap_or(Path::new(DEFAULT_HTML));
			write_report(Some(path), &report().to_html())?;
			ui.saved("HTML report", path);
		}
		Format::Junit => write_report(args.output.as_deref(), &report().to_junit())?,
	}

	Ok(exit_code(&summary, settings.strict))
}

async fn counterparty(lnd: &LndSettings, settings: &Settings) -> Result<Payments> {
	let node = Lnd::new(&lnd.url, &lnd.macaroon, lnd.cert.as_deref(), Duration::from_secs(60))?;
	node.check_access().await.context("the counterparty LND node can't be used")?;
	let info = node.info().await.context("failed to query the counterparty LND node")?;
	let payments = Payments::new(
		Box::new(node),
		info.network,
		settings.amount_sats * 1000,
		settings.max_sats * 1000,
	);
	Ok(payments.with_wallet_budget(settings.budget_sats.map(|sats| sats * 1000)))
}

fn init(args: InitArgs) -> Result<ExitCode> {
	if args.path.exists() && !args.force {
		bail!("{} already exists; pass --force to overwrite it", args.path.display());
	}
	let node = args.polar.as_deref().map(polar::find).transpose()?;
	std::fs::write(&args.path, config::template(node.as_ref()))
		.with_context(|| format!("failed to write {}", args.path.display()))?;

	let path = args.path.display();
	let mut out = format!("Wrote {path}");
	if let Some(name) = &args.polar {
		out.push_str(&format!(" with Polar node {name} as the counterparty"));
	}
	out.push_str(
		".\nSet allow = true under [spending] to run payment checks, then:\n\n  nwc-check run \"nostr+walletconnect://...\"\n",
	);
	ui::write_stdout(&out);
	Ok(ExitCode::SUCCESS)
}

const DEFAULT_HTML: &str = "nwc-check-report.html";

fn write_report(path: Option<&Path>, contents: &str) -> Result<()> {
	match path {
		Some(path) => {
			if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
				std::fs::create_dir_all(parent)
					.with_context(|| format!("failed to create {}", parent.display()))?;
			}
			std::fs::write(path, contents)
				.with_context(|| format!("failed to write the report to {}", path.display()))
		}
		None => {
			ui::write_stdout(contents);
			Ok(())
		}
	}
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
