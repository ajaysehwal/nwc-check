use std::time::Duration;

use reqwest::Url;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{Memo, MockNetwork, to_hex};

/// Serves a Lightning address on localhost whose invoices come from `network`, and returns
/// the address. Without `verify` the service has no LUD-21 verify URLs.
pub async fn serve_address(network: MockNetwork, verify: bool) -> String {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let host = listener.local_addr().unwrap().to_string();
	let base = format!("http://{host}");
	tokio::spawn(async move {
		while let Ok((stream, _)) = listener.accept().await {
			tokio::spawn(respond(stream, network.clone(), base.clone(), verify));
		}
	});
	format!("alice@{host}")
}

async fn respond(mut stream: TcpStream, network: MockNetwork, base: String, verify: bool) {
	let mut request = Vec::new();
	let mut buf = [0; 1024];
	while !request.windows(4).any(|window| window == b"\r\n\r\n") {
		match stream.read(&mut buf).await {
			Ok(0) | Err(_) => return,
			Ok(n) => request.extend_from_slice(&buf[..n]),
		}
	}
	let request = String::from_utf8_lossy(&request);
	let target = request.split_whitespace().nth(1).unwrap_or("/");
	let url = Url::parse(&format!("{base}{target}")).unwrap();
	let body = reply(&url, &network, &base, verify).to_string();
	let response = format!(
		"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
		body.len()
	);
	let _ = stream.write_all(response.as_bytes()).await;
}

fn reply(url: &Url, network: &MockNetwork, base: &str, verify: bool) -> Value {
	match url.path() {
		"/.well-known/lnurlp/alice" => json!({
			"tag": "payRequest",
			"callback": format!("{base}/callback?k=1"),
			"minSendable": 1000,
			"maxSendable": 100_000_000,
			"metadata": "[[\"text/plain\",\"nwc-check\"]]",
		}),
		"/callback" => {
			let amount = url.query_pairs().find(|(key, _)| key == "amount");
			let Some(amount) = amount.and_then(|(_, value)| value.parse().ok()) else {
				return json!({ "status": "ERROR", "reason": "no amount" });
			};
			let memo = Memo::Text("nwc-check".to_owned());
			let invoice = network.create(amount, memo, Duration::from_secs(3600));
			let hash = to_hex(&invoice.payment_hash);
			let verify = verify.then(|| format!("{base}/verify/{hash}"));
			json!({ "pr": invoice.bolt11, "routes": [], "verify": verify })
		}
		path => match path.strip_prefix("/verify/").and_then(|hash| network.find(Some(hash), None))
		{
			Some(invoice) => json!({
				"status": "OK",
				"settled": invoice.paid,
				"preimage": invoice.paid.then(|| to_hex(&invoice.preimage)),
				"pr": invoice.bolt11,
			}),
			None => json!({ "status": "ERROR", "reason": "not found" }),
		},
	}
}
