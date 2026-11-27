use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;

#[derive(Parser)]
#[command(
	name = "nwc-check",
	version,
	about = "Conformance and safety checks for NWC wallet services"
)]
pub struct Cli {
	#[command(subcommand)]
	pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
	/// Run the checks against a wallet service
	Run(Box<RunArgs>),
	/// Write a nwc-check.toml with every setting, so runs only need the connection string
	Init(InitArgs),
	/// List all checks
	List,
}

#[derive(clap::Args)]
pub struct RunArgs {
	/// Connection string (nostr+walletconnect://...)
	#[arg(env = "NWC_URI", hide_env_values = true)]
	pub uri: String,

	/// Second connection to the same wallet without pay_invoice permission, for SAFE-05
	#[arg(long, env = "NWC_READONLY_URI", hide_env_values = true, value_name = "URI")]
	pub readonly_uri: Option<String>,

	/// Settings file. Defaults to ./nwc-check.toml when it exists
	#[arg(long, env = "NWC_CHECK_CONFIG", value_name = "PATH")]
	pub config: Option<PathBuf>,

	/// Seconds to wait for each reply (default 10)
	#[arg(long, value_name = "SECS")]
	pub timeout: Option<u64>,

	/// Network the wallet is expected to report
	#[arg(long, value_enum)]
	pub network: Option<Network>,

	/// Only run checks whose ID starts with one of these prefixes, e.g. DISC,ENC-01
	#[arg(long, value_delimiter = ',', value_name = "IDS")]
	pub only: Vec<String>,

	/// Output format
	#[arg(long, value_enum, default_value_t = Format::Text)]
	pub format: Format,

	/// Treat warnings as failures
	#[arg(long)]
	pub strict: bool,

	/// File to write the report to (default nwc-check-report.html for html, stdout otherwise)
	#[arg(long, short, value_name = "PATH")]
	pub output: Option<PathBuf>,

	/// Disable colored output
	#[arg(long)]
	pub no_color: bool,

	#[command(flatten)]
	pub counterparty: CounterpartyArgs,
}

/// A counterparty that the wallet pays and that pays the wallet: an LND node, or a Lightning
/// address that can only be paid. Payment checks are skipped without one.
#[derive(clap::Args)]
#[command(next_help_heading = "Payment checks")]
pub struct CounterpartyArgs {
	/// LND REST URL of the counterparty node, e.g. https://localhost:8080
	#[arg(long, value_name = "URL")]
	pub lnd_url: Option<String>,

	/// LND macaroon file (admin.macaroon, or one that can create invoices and send payments)
	#[arg(long, value_name = "PATH")]
	pub lnd_macaroon: Option<PathBuf>,

	/// LND TLS certificate, for nodes with a self-signed cert
	#[arg(long, value_name = "PATH")]
	pub lnd_cert: Option<PathBuf>,

	/// Lightning address to pay instead of an LND node. It can't pay the wallet, so receive checks skip
	#[arg(long, value_name = "NAME@DOMAIN", conflicts_with = "lnd_url")]
	pub counterparty_address: Option<String>,

	/// Allow checks that move funds. They never run on mainnet
	#[arg(long)]
	pub allow_spend: bool,

	/// Amount for each test payment (default 10)
	#[arg(long, value_name = "SATS")]
	pub amount: Option<u64>,

	/// Most sats all payments in a run may move together (default 100)
	#[arg(long, value_name = "SATS")]
	pub max_sats: Option<u64>,

	/// Remaining budget of the connection, for wallets without get_budget
	#[arg(long, value_name = "SATS")]
	pub budget_sats: Option<u64>,
}

#[derive(clap::Args)]
pub struct InitArgs {
	/// Fill in the counterparty from this LND node in Polar, e.g. alice
	#[arg(long, value_name = "NODE")]
	pub polar: Option<String>,

	/// Where to write the settings
	#[arg(long, default_value = "nwc-check.toml", value_name = "PATH")]
	pub path: PathBuf,

	/// Overwrite an existing file
	#[arg(long)]
	pub force: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
	Text,
	Json,
	/// Terminal output, plus a self-contained page to share
	Html,
	/// JUnit XML, for CI systems that show results per test
	Junit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
	Mainnet,
	Testnet,
	Signet,
	Regtest,
}

impl Network {
	pub fn as_str(self) -> &'static str {
		match self {
			Self::Mainnet => "mainnet",
			Self::Testnet => "testnet",
			Self::Signet => "signet",
			Self::Regtest => "regtest",
		}
	}
}
