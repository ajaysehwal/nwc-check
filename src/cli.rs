use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

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
	Run(RunArgs),
	/// List all checks
	List,
}

#[derive(clap::Args)]
pub struct RunArgs {
	/// Connection string (nostr+walletconnect://...)
	#[arg(env = "NWC_URI", hide_env_values = true)]
	pub uri: String,

	/// Seconds to wait for each reply
	#[arg(long, default_value_t = 10, value_name = "SECS")]
	pub timeout: u64,

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

	/// Disable colored output
	#[arg(long)]
	pub no_color: bool,

	#[command(flatten)]
	pub counterparty: CounterpartyArgs,
}

/// A node you control that pays and gets paid by the wallet. Payment checks are skipped without one.
#[derive(clap::Args)]
#[command(next_help_heading = "Payment checks")]
pub struct CounterpartyArgs {
	/// LND REST URL of the counterparty node, e.g. https://localhost:8080
	#[arg(long, value_name = "URL", requires = "lnd_macaroon")]
	pub lnd_url: Option<String>,

	/// LND macaroon file (admin.macaroon, or one that can create invoices and send payments)
	#[arg(long, value_name = "PATH")]
	pub lnd_macaroon: Option<PathBuf>,

	/// LND TLS certificate, for nodes with a self-signed cert
	#[arg(long, value_name = "PATH")]
	pub lnd_cert: Option<PathBuf>,

	/// Allow checks that move funds. They never run on mainnet
	#[arg(long)]
	pub allow_spend: bool,

	/// Amount for each test payment
	#[arg(long, default_value_t = 10, value_name = "SATS")]
	pub amount: u64,

	/// Most sats all payments in a run may move together
	#[arg(long, default_value_t = 100, value_name = "SATS")]
	pub max_sats: u64,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
	Text,
	Json,
}

#[derive(Clone, Copy, ValueEnum)]
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
