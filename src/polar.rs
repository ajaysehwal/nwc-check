use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

pub struct PolarNode {
	pub url: String,
	pub macaroon: PathBuf,
	pub cert: PathBuf,
}

pub fn find(name: &str) -> Result<PolarNode> {
	let home = std::env::var_os("HOME").context("HOME isn't set")?;
	let index = PathBuf::from(home).join(".polar/networks/networks.json");
	let text = std::fs::read_to_string(&index)
		.with_context(|| format!("couldn't read Polar's networks at {}", index.display()))?;
	let index: Value = serde_json::from_str(&text).context("unexpected Polar networks file")?;

	let mut names = Vec::new();
	for network in index["networks"].as_array().into_iter().flatten() {
		for node in network["nodes"]["lightning"].as_array().into_iter().flatten() {
			if node["implementation"] != "LND" {
				continue;
			}
			let node_name = node["name"].as_str().unwrap_or_default();
			if node_name != name {
				names.push(node_name.to_owned());
				continue;
			}
			let (Some(path), Some(port)) =
				(network["path"].as_str(), node["ports"]["rest"].as_u64())
			else {
				bail!("Polar node {name} has no path or REST port");
			};
			let dir = PathBuf::from(path).join("volumes/lnd").join(name);
			return Ok(PolarNode {
				url: format!("https://127.0.0.1:{port}"),
				macaroon: dir.join("data/chain/bitcoin/regtest/admin.macaroon"),
				cert: dir.join("tls.cert"),
			});
		}
	}
	if names.is_empty() {
		bail!("no LND nodes found in Polar");
	}
	bail!("no LND node named {name} in Polar; found {}", names.join(", "))
}
