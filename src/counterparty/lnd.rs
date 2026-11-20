use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures::FutureExt;
use futures::future::BoxFuture;
use reqwest::{Certificate, Client, Method, RequestBuilder};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use super::{Counterparty, Invoice, NodeInfo, to_hex};

pub struct Lnd {
	http: Client,
	url: String,
	macaroon: String,
}

impl Lnd {
	pub fn new(
		url: &str,
		macaroon: &Path,
		tls_cert: Option<&Path>,
		timeout: Duration,
	) -> Result<Self> {
		let macaroon = std::fs::read(macaroon)
			.with_context(|| format!("failed to read macaroon {}", macaroon.display()))?;
		let mut builder = Client::builder().timeout(timeout);
		if let Some(path) = tls_cert {
			let pem = std::fs::read(path)
				.with_context(|| format!("failed to read TLS cert {}", path.display()))?;
			builder = builder.add_root_certificate(Certificate::from_pem(&pem)?);
		}
		Ok(Self {
			http: builder.build()?,
			url: url.trim_end_matches('/').to_owned(),
			macaroon: to_hex(&macaroon),
		})
	}

	pub async fn check_access(&self) -> Result<()> {
		// Invalid requests: LND rejects them without side effects, and says "permission denied" first if the macaroon lacks access.
		let probes = [
			("/v1/invoices", json!({ "value_msat": "-1" }), "create invoices"),
			("/v1/channels/transactions", json!({ "payment_request": "" }), "send payments"),
		];
		for (path, body, action) in probes {
			let reply = self.request(Method::POST, path).json(&body).send().await?.text().await?;
			if reply.contains("permission denied") {
				bail!(
					"the macaroon can't {action}; use admin.macaroon or one with info:read, invoices:write and offchain:write"
				);
			}
		}
		Ok(())
	}

	fn request(&self, method: Method, path: &str) -> RequestBuilder {
		self.http
			.request(method, format!("{}{path}", self.url))
			.header("Grpc-Metadata-macaroon", &self.macaroon)
	}

	async fn call<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T> {
		let response = request.send().await?;
		let status = response.status();
		let body = response.text().await?;
		if !status.is_success() {
			let message =
				serde_json::from_str::<ErrorBody>(&body).map(|error| error.message).unwrap_or(body);
			bail!("LND returned {status}: {message}");
		}
		serde_json::from_str(&body).context("unexpected reply from LND")
	}

	async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
		self.call(self.request(Method::GET, path)).await
	}

	async fn post<T: DeserializeOwned>(&self, path: &str, body: serde_json::Value) -> Result<T> {
		self.call(self.request(Method::POST, path).json(&body)).await
	}
}

impl Counterparty for Lnd {
	fn info(&self) -> BoxFuture<'_, Result<NodeInfo>> {
		async move {
			let info: GetInfo = self.get("/v1/getinfo").await?;
			let network = info.chains.into_iter().next().map(|chain| chain.network);
			Ok(NodeInfo {
				pubkey: info.identity_pubkey,
				network: network.ok_or_else(|| anyhow!("LND reported no chain"))?,
			})
		}
		.boxed()
	}

	fn create_invoice<'a>(
		&'a self,
		amount_msat: u64,
		memo: &'a str,
	) -> BoxFuture<'a, Result<Invoice>> {
		async move {
			let body = json!({ "value_msat": amount_msat.to_string(), "memo": memo });
			let invoice: AddInvoice = self.post("/v1/invoices", body).await?;
			let payment_hash = STANDARD
				.decode(&invoice.r_hash)?
				.try_into()
				.map_err(|_| anyhow!("LND returned a payment hash that isn't 32 bytes"))?;
			Ok(Invoice { bolt11: invoice.payment_request, payment_hash })
		}
		.boxed()
	}

	fn is_settled<'a>(&'a self, payment_hash: &'a [u8; 32]) -> BoxFuture<'a, Result<bool>> {
		async move {
			let invoice: LookupInvoice =
				self.get(&format!("/v1/invoice/{}", to_hex(payment_hash))).await?;
			Ok(invoice.state == "SETTLED")
		}
		.boxed()
	}

	fn pay<'a>(&'a self, bolt11: &'a str) -> BoxFuture<'a, Result<()>> {
		async move {
			let body = json!({ "payment_request": bolt11 });
			let payment: SendPayment = self.post("/v1/channels/transactions", body).await?;
			match payment.payment_error.filter(|error| !error.is_empty()) {
				Some(error) => bail!("payment failed: {error}"),
				None => Ok(()),
			}
		}
		.boxed()
	}
}

#[derive(Deserialize)]
struct ErrorBody {
	message: String,
}

#[derive(Deserialize)]
struct GetInfo {
	identity_pubkey: String,
	chains: Vec<Chain>,
}

#[derive(Deserialize)]
struct Chain {
	network: String,
}

#[derive(Deserialize)]
struct AddInvoice {
	r_hash: String,
	payment_request: String,
}

#[derive(Deserialize)]
struct LookupInvoice {
	state: String,
}

#[derive(Deserialize)]
struct SendPayment {
	payment_error: Option<String>,
}
