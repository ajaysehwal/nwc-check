use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use futures::FutureExt;
use futures::future::BoxFuture;
use lightning_invoice::{Bolt11Invoice, Currency};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{Counterparty, Invoice, NodeInfo, PayError};

/// A Lightning address (LUD-16) the wallet pays. It gets invoices over LNURL-pay (LUD-06) and
/// confirms payments through LUD-21 verify URLs, when the service offers them. It can't pay the
/// wallet, and it only learns that an invoice was paid, not how much arrived.
pub struct LightningAddress {
	http: Client,
	address: String,
	domain: String,
	url: Url,
	issued: Mutex<HashMap<[u8; 32], Issued>>,
}

struct Issued {
	verify: Option<Url>,
	amount_msat: u64,
}

impl LightningAddress {
	pub fn new(address: &str, timeout: Duration) -> Result<Self> {
		let address = address.trim().to_lowercase();
		let invalid = || anyhow!("{address} isn't a Lightning address like name@example.com");
		let (user, domain) = address.split_once('@').ok_or_else(invalid)?;
		let user_ok = !user.is_empty()
			&& user.chars().all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c));
		let domain_ok = !domain.is_empty() && !domain.contains(['/', '?', '#', '@']);
		ensure!(user_ok && domain_ok, invalid());

		let mut url = Url::parse(&format!("https://{domain}/.well-known/lnurlp/{user}"))
			.map_err(|_| invalid())?;
		// LUD-16 allows plain HTTP for onion services; local hosts get it too, for regtest setups.
		let host = url.host_str().unwrap_or_default();
		if ["localhost", "127.0.0.1", "[::1]"].contains(&host) || host.ends_with(".onion") {
			url.set_scheme("http").map_err(|_| invalid())?;
		}
		Ok(Self {
			http: Client::builder().timeout(timeout).build()?,
			domain: domain.to_owned(),
			address: address.clone(),
			url,
			issued: Mutex::default(),
		})
	}

	pub fn address(&self) -> &str {
		&self.address
	}

	/// The service every request goes to.
	pub fn domain(&self) -> &str {
		&self.domain
	}

	async fn get<T: DeserializeOwned>(&self, url: Url) -> Result<T> {
		let reply: Value = self.http.get(url).send().await?.error_for_status()?.json().await?;
		if reply.get("status").and_then(Value::as_str) == Some("ERROR") {
			let reason = reply.get("reason").and_then(Value::as_str).unwrap_or("no reason given");
			bail!("{} returned an error: {reason}", self.domain);
		}
		serde_json::from_value(reply)
			.with_context(|| format!("unexpected reply from {}", self.domain))
	}

	async fn pay_request(&self) -> Result<PayRequest> {
		let request: PayRequest = self.get(self.url.clone()).await?;
		ensure!(request.tag == "payRequest", "{} isn't a LNURL-pay endpoint", self.address);
		Ok(request)
	}

	async fn invoice(&self, request: &PayRequest, amount_msat: u64) -> Result<Bolt11Invoice> {
		ensure!(amount_msat > 0, "a Lightning address can't give an invoice without an amount");
		ensure!(
			(request.min_sendable..=request.max_sendable).contains(&amount_msat),
			"{} accepts {} to {} msats, not {amount_msat}",
			self.address,
			request.min_sendable,
			request.max_sendable
		);
		let mut callback = Url::parse(&request.callback)
			.with_context(|| format!("{} gave an invalid callback URL", self.domain))?;
		callback.query_pairs_mut().append_pair("amount", &amount_msat.to_string());
		let reply: Callback = self.get(callback).await?;

		let invoice = Bolt11Invoice::from_str(&reply.pr)
			.map_err(|error| anyhow!("{} returned an invalid invoice: {error}", self.domain))?;
		ensure!(
			invoice.amount_milli_satoshis() == Some(amount_msat),
			"{} returned an invoice for a different amount",
			self.domain
		);
		let verify = reply.verify.map(|url| Url::parse(&url)).transpose()?;
		let hash = payment_hash(&invoice);
		let mut issued = self.issued.lock().unwrap_or_else(|e| e.into_inner());
		issued.insert(hash, Issued { verify, amount_msat });
		Ok(invoice)
	}
}

impl Counterparty for LightningAddress {
	fn info(&self) -> BoxFuture<'_, Result<NodeInfo>> {
		async move {
			// The address only says which network it's on through the invoices it gives out.
			let request = self.pay_request().await?;
			let invoice = self.invoice(&request, request.min_sendable.max(1)).await?;
			Ok(NodeInfo {
				pubkey: invoice.get_payee_pub_key().to_string(),
				network: network(invoice.currency()).to_owned(),
			})
		}
		.boxed()
	}

	fn create_invoice<'a>(
		&'a self,
		amount_msat: u64,
		_memo: &'a str,
	) -> BoxFuture<'a, Result<Invoice>> {
		async move {
			let request = self.pay_request().await?;
			let invoice = self.invoice(&request, amount_msat).await?;
			let payment_hash = payment_hash(&invoice);
			Ok(Invoice { bolt11: invoice.to_string(), payment_hash })
		}
		.boxed()
	}

	fn paid_msat<'a>(&'a self, payment_hash: &'a [u8; 32]) -> BoxFuture<'a, Result<Option<u64>>> {
		async move {
			let (verify, amount_msat) = {
				let issued = self.issued.lock().unwrap_or_else(|e| e.into_inner());
				let issued = issued
					.get(payment_hash)
					.with_context(|| format!("{} didn't issue this payment", self.address))?;
				(issued.verify.clone(), issued.amount_msat)
			};
			let Some(verify) = verify else {
				bail!("{} has no LUD-21 verify URL, so payments can't be confirmed", self.domain);
			};
			let status: Verify = self.get(verify).await?;
			Ok(status.settled.then_some(amount_msat))
		}
		.boxed()
	}

	fn pay<'a>(&'a self, _bolt11: &'a str) -> BoxFuture<'a, Result<(), PayError>> {
		async { Err(PayError::Unable("a Lightning address can't pay the wallet".to_owned())) }
			.boxed()
	}

	fn receives_keysend(&self) -> bool {
		false
	}

	fn sees_amount_received(&self) -> bool {
		false
	}
}

fn payment_hash(invoice: &Bolt11Invoice) -> [u8; 32] {
	let mut hash = [0; 32];
	hash.copy_from_slice(invoice.payment_hash().as_ref());
	hash
}

fn network(currency: Currency) -> &'static str {
	match currency {
		Currency::Bitcoin => "mainnet",
		Currency::BitcoinTestnet => "testnet",
		Currency::Regtest => "regtest",
		Currency::Simnet => "simnet",
		Currency::Signet => "signet",
	}
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PayRequest {
	tag: String,
	callback: String,
	min_sendable: u64,
	max_sendable: u64,
}

#[derive(Deserialize)]
struct Callback {
	pr: String,
	verify: Option<String>,
}

#[derive(Deserialize)]
struct Verify {
	settled: bool,
}

#[cfg(test)]
mod tests {
	use super::*;

	fn url(address: &str) -> Result<String> {
		Ok(LightningAddress::new(address, Duration::from_secs(1))?.url.to_string())
	}

	#[test]
	fn addresses_map_to_lnurlp_urls() {
		assert_eq!(
			url("Alice@Example.com").unwrap(),
			"https://example.com/.well-known/lnurlp/alice"
		);
		assert_eq!(
			url("bob@127.0.0.1:3000").unwrap(),
			"http://127.0.0.1:3000/.well-known/lnurlp/bob"
		);
		assert!(url("carol@abc.onion").unwrap().starts_with("http://"));
	}

	#[test]
	fn malformed_addresses_are_rejected() {
		for address in ["alice", "@example.com", "alice@", "a/b@example.com", "a@b/c", "a@b@c"] {
			assert!(url(address).is_err(), "{address}");
		}
	}
}
