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
