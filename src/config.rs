use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::cli::{Network, RunArgs};
use crate::polar::PolarNode;

pub const DEFAULT_PATH: &str = "nwc-check.toml";

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
	pub timeout: Option<u64>,
	pub network: Option<Network>,
	pub only: Option<Vec<String>>,
	pub strict: Option<bool>,
	pub with_flood: Option<bool>,
	pub lnd: LndConfig,
	pub spending: SpendingConfig,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LndConfig {
	pub url: Option<String>,
	pub macaroon: Option<PathBuf>,
	pub cert: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpendingConfig {
	pub allow: Option<bool>,
	pub amount: Option<u64>,
	pub max_sats: Option<u64>,
	pub budget_sats: Option<u64>,
}

impl Config {
	pub fn load(explicit: Option<&Path>) -> Result<Option<(PathBuf, Self)>> {
		let path = match explicit {
			Some(path) => path.to_owned(),
			None if Path::new(DEFAULT_PATH).exists() => PathBuf::from(DEFAULT_PATH),
			None => return Ok(None),
		};
		let text = std::fs::read_to_string(&path)
			.with_context(|| format!("failed to read {}", path.display()))?;
		let config = Self::parse(&text, path.parent().unwrap_or(Path::new("")))
			.with_context(|| format!("invalid settings in {}", path.display()))?;
		Ok(Some((path, config)))
	}

	fn parse(text: &str, base: &Path) -> Result<Self> {
		let mut config: Self = toml::from_str(text)?;
		config.lnd.macaroon = config.lnd.macaroon.map(|path| resolve(base, path));
		config.lnd.cert = config.lnd.cert.map(|path| resolve(base, path));
		Ok(config)
	}
}

fn resolve(base: &Path, path: PathBuf) -> PathBuf {
	if let (Ok(rest), Some(home)) = (path.strip_prefix("~"), std::env::var_os("HOME")) {
		return PathBuf::from(home).join(rest);
	}
	if path.is_relative() { base.join(path) } else { path }
}

#[derive(Debug)]
pub struct Settings {
	pub timeout: Duration,
	pub network: Option<Network>,
	pub only: Vec<String>,
	pub strict: bool,
	pub with_flood: bool,
	pub lnd: Option<LndSettings>,
	pub allow_spend: bool,
	pub amount_sats: u64,
	pub max_sats: u64,
	pub budget_sats: Option<u64>,
}

#[derive(Debug)]
pub struct LndSettings {
	pub url: String,
	pub macaroon: PathBuf,
	pub cert: Option<PathBuf>,
}

impl Settings {
	pub fn resolve(args: &RunArgs, config: Config) -> Result<Self> {
		let cli = &args.counterparty;
		let lnd = match cli.lnd_url.clone().or(config.lnd.url) {
			None => None,
			Some(url) => Some(LndSettings {
				url,
				macaroon: cli.lnd_macaroon.clone().or(config.lnd.macaroon).context(
					"an LND URL needs a macaroon: pass --lnd-macaroon or set macaroon under [lnd]",
				)?,
				cert: cli.lnd_cert.clone().or(config.lnd.cert),
			}),
		};
		let only =
			if args.only.is_empty() { config.only.unwrap_or_default() } else { args.only.clone() };
		let settings = Self {
			timeout: Duration::from_secs(args.timeout.or(config.timeout).unwrap_or(10)),
			network: args.network.or(config.network),
			only,
			strict: args.strict || config.strict.unwrap_or(false),
			with_flood: args.with_flood || config.with_flood.unwrap_or(false),
			lnd,
			allow_spend: cli.allow_spend || config.spending.allow.unwrap_or(false),
			amount_sats: cli.amount.or(config.spending.amount).unwrap_or(10),
			max_sats: cli.max_sats.or(config.spending.max_sats).unwrap_or(100),
			budget_sats: cli.budget_sats.or(config.spending.budget_sats),
		};
		if settings.amount_sats == 0 {
			bail!("the payment amount must be at least 1 sat");
		}
		Ok(settings)
	}
}

pub fn template(polar: Option<&PolarNode>) -> String {
	let quote = |path: &Path| toml::Value::String(path.display().to_string()).to_string();
	let (network, lnd) = match polar {
		Some(node) => (
			"network = \"regtest\"".to_owned(),
			format!(
				"url = {}\nmacaroon = {}\ncert = {}",
				toml::Value::String(node.url.clone()),
				quote(&node.macaroon),
				quote(&node.cert)
			),
		),
		None => (
			"# network = \"regtest\"".to_owned(),
			"# url = \"https://localhost:8080\"\n# macaroon = \"~/.lnd/data/chain/bitcoin/regtest/admin.macaroon\"\n# cert = \"~/.lnd/tls.cert\"".to_owned(),
		),
	};
	format!(
		r#"# nwc-check settings. Command-line options override anything set here.
# The connection string is a secret and isn't stored here: pass it to
# `nwc-check run` or set NWC_URI.

# Seconds to wait for each reply
timeout = 10

# Network the wallet should report: testnet, signet or regtest
{network}

# Only run checks whose ID starts with one of these prefixes
# only = ["DISC", "CORE"]

# Treat warnings as failures
strict = false

# Run SAFE-15, which sends a burst of 60 get_info requests to test rate
# limiting. Off by default, since it can trip alerts on hosted wallets.
with_flood = false

# Counterparty node for payment checks. Paths can be relative to this file
# or start with ~/.
[lnd]
{lnd}

[spending]
# Allow checks that move funds. They never run on mainnet.
allow = false
# Amount of each test payment, in sats
amount = 10
# Most sats one run may move in total
max_sats = 100
# Remaining budget of this connection, for wallets without get_budget.
# SAFE-02 uses it up.
# budget_sats = 50
"#
	)
}

#[cfg(test)]
mod tests {
	use clap::Parser;

	use super::*;
	use crate::cli::{Cli, Command};

	fn args(extra: &[&str]) -> RunArgs {
		let argv = ["nwc-check", "run", "nostr+walletconnect://x"].iter().chain(extra);
		match Cli::try_parse_from(argv).unwrap().command {
			Command::Run(args) => *args,
			_ => unreachable!(),
		}
	}

	#[test]
	fn templates_are_valid_settings() {
		let node = PolarNode {
			url: "https://127.0.0.1:8082".to_owned(),
			macaroon: PathBuf::from("/p/admin.macaroon"),
			cert: PathBuf::from("/p/tls.cert"),
		};
		let plain = Config::parse(&template(None), Path::new("")).unwrap();
		assert!(plain.lnd.url.is_none());
		let filled = Config::parse(&template(Some(&node)), Path::new("")).unwrap();
		assert_eq!(filled.lnd.url.as_deref(), Some("https://127.0.0.1:8082"));
		assert_eq!(filled.network, Some(Network::Regtest));
	}

	#[test]
	fn command_line_overrides_the_file() {
		let config = Config::parse(
			"timeout = 30\nonly = [\"DISC\"]\n[lnd]\nurl = \"https://file\"\nmacaroon = \"m\"\n[spending]\nallow = true\namount = 5",
			Path::new("/conf"),
		)
		.unwrap();
		let settings =
			Settings::resolve(&args(&["--timeout", "3", "--lnd-url", "https://cli"]), config)
				.unwrap();
		assert_eq!(settings.timeout, Duration::from_secs(3));
		assert_eq!(settings.only, ["DISC"]);
		assert!(settings.allow_spend);
		assert_eq!(settings.amount_sats, 5);
		assert_eq!(settings.max_sats, 100);
		let lnd = settings.lnd.unwrap();
		assert_eq!(lnd.url, "https://cli");
		assert_eq!(lnd.macaroon, PathBuf::from("/conf/m"));
	}

	#[test]
	fn lnd_url_without_macaroon_is_an_error() {
		assert!(
			Settings::resolve(&args(&["--lnd-url", "https://cli"]), Config::default()).is_err()
		);
	}

	#[test]
	fn unknown_keys_are_rejected() {
		assert!(Config::parse("timout = 3", Path::new("")).is_err());
	}
}
