use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use nwc_check::counterparty::{Counterparty, from_hex, to_hex};
use reqwest::Url;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A Lightning address on localhost whose invoices come from `node`. It speaks just enough
/// LNURL-pay (LUD-06, LUD-16) and LUD-21 verify for nwc-check.
pub struct AddressServer {
	node: Box<dyn Counterparty>,
	name: String,
	base: String,
	verify: bool,
	issued: Mutex<HashMap<String, String>>,
}

impl AddressServer {
	/// Serves `name` on 127.0.0.1:`port` (0 picks a free port) and returns the address. Without
	/// `verify` the server gives no LUD-21 verify URLs.
	pub async fn start(
		node: Box<dyn Counterparty>,
		name: &str,
		port: u16,
		verify: bool,
	) -> std::io::Result<String> {
		let listener = TcpListener::bind(("127.0.0.1", port)).await?;
		let host = listener.local_addr()?.to_string();
		let server = Arc::new(Self {
			node,
			name: name.to_owned(),
			base: format!("http://{host}"),
			verify,
			issued: Mutex::default(),
		});
		tokio::spawn(async move {
			while let Ok((stream, _)) = listener.accept().await {
				tokio::spawn(server.clone().respond(stream));
			}
		});
		Ok(format!("{name}@{host}"))
	}

	async fn respond(self: Arc<Self>, mut stream: TcpStream) {
		let mut request = Vec::new();
		let mut buf = [0; 1024];
		while !request.windows(4).any(|window| window == b"\r\n\r\n") {
			match stream.read(&mut buf).await {
				Ok(0) | Err(_) => return,
				Ok(n) => request.extend_from_slice(&buf[..n]),
			}
		}
		let request = String::from_utf8_lossy(&request).into_owned();
		let target = request.split_whitespace().nth(1).unwrap_or("/");
		let body = match Url::parse(&format!("{}{target}", self.base)) {
			Ok(url) => self.reply(&url).await,
			Err(_) => error("bad request"),
		}
		.to_string();
		let response = format!(
			"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
			body.len()
		);
		let _ = stream.write_all(response.as_bytes()).await;
	}

	async fn reply(&self, url: &Url) -> Value {
		let path = url.path();
		if path == format!("/.well-known/lnurlp/{}", self.name) {
			return json!({
				"tag": "payRequest",
				"callback": format!("{}/callback", self.base),
				"minSendable": 1000,
				"maxSendable": 1_000_000_000,
				"metadata": "[[\"text/plain\",\"nwc-check\"]]",
			});
		}
		if path == "/callback" {
			let amount = url.query_pairs().find(|(key, _)| key == "amount");
			let Some(amount) = amount.and_then(|(_, value)| value.parse().ok()) else {
				return error("no amount");
			};
			let invoice = match self.node.create_invoice(amount, "nwc-check").await {
				Ok(invoice) => invoice,
				Err(e) => return error(&format!("{e:#}")),
			};
			let hash = to_hex(&invoice.payment_hash);
			self.issued.lock().unwrap().insert(hash.clone(), invoice.bolt11.clone());
			let verify = self.verify.then(|| format!("{}/verify/{hash}", self.base));
			return json!({ "pr": invoice.bolt11, "routes": [], "verify": verify });
		}
		let Some(hash) = path.strip_prefix("/verify/").filter(|_| self.verify) else {
			return error("not found");
		};
		let Some(pr) = self.issued.lock().unwrap().get(hash).cloned() else {
			return error("not found");
		};
		let Some(bytes) = from_hex(hash).and_then(|bytes| <[u8; 32]>::try_from(bytes).ok()) else {
			return error("not found");
		};
		match self.node.paid_msat(&bytes).await {
			Ok(paid) => {
				json!({ "status": "OK", "settled": paid.is_some(), "preimage": null, "pr": pr })
			}
			Err(e) => error(&format!("{e:#}")),
		}
	}
}

fn error(reason: &str) -> Value {
	json!({ "status": "ERROR", "reason": reason })
}
