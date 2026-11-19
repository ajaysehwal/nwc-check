#[path = "../tests/support/mod.rs"]
mod support;

use nostr_sdk::prelude::MockRelay;

use crate::support::{Bugs, MockWallet};

#[tokio::main]
async fn main() {
	let buggy = std::env::args().any(|arg| arg == "--buggy");
	let bugs = if buggy {
		Bugs { answer_expired: true, wrong_error_code: true, skip_p_tag: true }
	} else {
		Bugs::default()
	};

	let relay = MockRelay::run().await.expect("failed to start the local relay");
	let uri = MockWallet::start(&relay, bugs).await;
	println!("{uri}");
	eprintln!(
		"Mock wallet running{}. Press Ctrl-C to stop.",
		if buggy { " with bugs" } else { "" }
	);

	tokio::signal::ctrl_c().await.expect("failed to listen for Ctrl-C");
}
