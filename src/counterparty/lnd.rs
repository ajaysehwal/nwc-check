use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures::FutureExt;
use futures::future::BoxFuture;
use reqwest::{Certificate, Client, Method, RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use super::{Counterparty, Invoice, NodeInfo, PayError, to_hex};

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
		parse(request.send().await?).await
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

	fn paid_msat<'a>(&'a self, payment_hash: &'a [u8; 32]) -> BoxFuture<'a, Result<Option<u64>>> {
		async move {
			let path = format!("/v1/invoice/{}", to_hex(payment_hash));
			// LND only records a keysend once it arrives, so an unknown hash means unpaid.
			let response = self.request(Method::GET, &path).send().await?;
			if response.status() == StatusCode::NOT_FOUND {
				return Ok(None);
			}
			let invoice: LookupInvoice = parse(response).await?;
			if invoice.state != "SETTLED" {
				return Ok(None);
			}
			Ok(Some(invoice.amt_paid_msat.parse().context("LND returned an invalid amount")?))
		}
		.boxed()
	}

	fn pay<'a>(&'a self, bolt11: &'a str) -> BoxFuture<'a, Result<(), PayError>> {
		async move {
			let unable = |error: reqwest::Error| PayError::Unable(error.to_string());
			let response = self
				.request(Method::POST, "/v1/channels/transactions")
				.json(&json!({ "payment_request": bolt11 }))
				.send()
				.await
				.map_err(unable)?;
			let status = response.status();
			let body = response.text().await.map_err(unable)?;
			if !status.is_success() {
				let message = error_message(body);
				return Err(if message.contains("permission denied") {
					PayError::Unable(message)
				} else {
					PayError::Failed(format!("LND rejected the invoice: {message}"))
				});
			}
			let payment: SendPayment = serde_json::from_str(&body)
				.map_err(|_| PayError::Unable("unexpected reply from LND".to_owned()))?;
			match payment.payment_error.filter(|error| !error.is_empty()) {
				Some(error) if error.contains("insufficient_balance") => {
					Err(PayError::Unable(error))
				}
				Some(error) => Err(PayError::Failed(error)),
				None => Ok(()),
			}
		}
		.boxed()
	}
}

async fn parse<T: DeserializeOwned>(response: Response) -> Result<T> {
	let status = response.status();
	let body = response.text().await?;
	if !status.is_success() {
		bail!("LND returned {status}: {}", error_message(body));
	}
	serde_json::from_str(&body).context("unexpected reply from LND")
}

fn error_message(body: String) -> String {
	serde_json::from_str::<ErrorBody>(&body).map(|error| error.message).unwrap_or(body)
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
	#[serde(default)]
	amt_paid_msat: String,
}

#[derive(Deserialize)]
struct SendPayment {
	payment_error: Option<String>,
}
