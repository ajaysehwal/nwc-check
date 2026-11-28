#[path = "../tests/support/lnurl.rs"]
mod lnurl;

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use nwc_check::counterparty::{Counterparty, Lnd};

use crate::lnurl::AddressServer;

/// Serves a Lightning address on localhost whose invoices come from an LND node, so
/// --counterparty-address can be tried on regtest.
#[derive(Parser)]
struct Args {
	/// LND REST URL of the node that receives the payments
	#[arg(long)]
	lnd_url: String,
	/// Macaroon that can create and read invoices
	#[arg(long)]
	lnd_macaroon: PathBuf,
	/// TLS certificate, for nodes with a self-signed cert
	#[arg(long)]
	lnd_cert: Option<PathBuf>,
	/// Name before the @
	#[arg(long, default_value = "bob")]
	name: String,
	#[arg(long, default_value_t = 3000)]
	port: u16,
	/// Leave out LUD-21 verify URLs, like services that don't support it
	#[arg(long)]
	no_verify: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let args = Args::parse();
	let node = Lnd::new(
		&args.lnd_url,
		&args.lnd_macaroon,
		args.lnd_cert.as_deref(),
		Duration::from_secs(30),
	)?;
	let info = node.info().await?;
	let address =
		AddressServer::start(Box::new(node), &args.name, args.port, !args.no_verify).await;
	println!("{address}");
	eprintln!(
		"Serving {address} from LND on {}{}. Press Ctrl-C to stop.",
		info.network,
		if args.no_verify { ", without verify" } else { "" }
	);
	tokio::signal::ctrl_c().await?;
	Ok(())
}
