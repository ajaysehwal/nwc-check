mod support;

use std::time::Duration;

use nostr_sdk::prelude::MockRelay;
use nwc_check::check::{Outcome, Status};
use nwc_check::checks::{self, Context, Payments};
use nwc_check::wallet::Wallet;

use crate::support::{Bugs, MockCounterparty, MockNetwork, MockWallet};

const PAYMENT_CHECKS: [&str; 4] = ["PAY-01", "PAY-02", "PAY-03", "CORE-04"];
const INVOICE_CHECKS: [&str; 5] = ["CORE-01", "CORE-02", "CORE-03", "CORE-04", "CORE-05"];

struct Run {
	bugs: Bugs,
	chain: &'static str,
	counterparty: bool,
	broke_counterparty: bool,
	max_sats: u64,
}

impl Default for Run {
	fn default() -> Self {
		Self {
			bugs: Bugs::default(),
			chain: "regtest",
			counterparty: true,
			broke_counterparty: false,
			max_sats: 100,
		}
	}
}

impl Run {
	async fn checks(self) -> (Vec<Outcome>, MockNetwork) {
		let relay = MockRelay::run().await.unwrap();
		let network = MockNetwork::new(self.chain);
		let uri = MockWallet::start(&relay, self.bugs, network.clone()).await;
		let wallet = Wallet::connect(uri, Duration::from_secs(3)).await.unwrap();
		let mut ctx = Context::load(wallet, Some(self.chain.to_owned())).await.unwrap();
		if self.counterparty {
			let node = Box::new(MockCounterparty {
				network: network.clone(),
				broke: self.broke_counterparty,
			});
			let payments = Payments::new(node, self.chain.to_owned(), 10_000, self.max_sats * 1000);
			ctx = ctx.with_payments(payments);
		}
		(checks::run(&ctx).await, network)
	}
}

fn status(outcomes: &[Outcome], id: &str) -> Status {
	outcomes.iter().find(|outcome| outcome.id == id).map(|outcome| outcome.status).unwrap()
}

#[tokio::test]
async fn compliant_wallet_has_no_failures() {
	let (outcomes, _) = Run::default().checks().await;
	let problems: Vec<&Outcome> = outcomes.iter().filter(|outcome| outcome.is_problem()).collect();
	assert!(problems.is_empty(), "{problems:#?}");
	assert_eq!(outcomes.len(), checks::catalog().len());
	for id in PAYMENT_CHECKS.iter().chain(&INVOICE_CHECKS) {
		assert_eq!(status(&outcomes, id), Status::Pass, "{id}");
	}
}

#[tokio::test]
async fn payment_checks_skip_without_counterparty() {
	let (outcomes, network) = Run { counterparty: false, ..Run::default() }.checks().await;
	for id in PAYMENT_CHECKS {
		assert_eq!(status(&outcomes, id), Status::Skip, "{id}");
	}
	assert_eq!(network.moved_msat(), 0);
}

#[tokio::test]
async fn mainnet_wallet_spends_nothing() {
	let (outcomes, network) = Run { chain: "mainnet", ..Run::default() }.checks().await;
	for id in PAYMENT_CHECKS {
		assert_eq!(status(&outcomes, id), Status::Skip, "{id}");
	}
	assert_eq!(network.moved_msat(), 0);
}

#[tokio::test]
async fn payments_stop_at_max_sats() {
	let (outcomes, network) = Run { max_sats: 15, ..Run::default() }.checks().await;
	let over_budget = outcomes.iter().filter(|outcome| outcome.detail.contains("--max-sats"));
	assert!(over_budget.count() >= 2);
	assert_eq!(network.moved_msat(), 10_000);
}

#[tokio::test]
async fn counterparty_without_funds_isnt_blamed_on_the_wallet() {
	let (outcomes, _) = Run { broke_counterparty: true, ..Run::default() }.checks().await;
	let problems: Vec<&Outcome> = outcomes.iter().filter(|outcome| outcome.is_problem()).collect();
	assert!(problems.is_empty(), "{problems:#?}");
	assert_eq!(status(&outcomes, "PAY-01"), Status::Skip);
	assert_eq!(status(&outcomes, "PAY-02"), Status::Pass);
}

#[tokio::test]
async fn buggy_wallet_is_caught() {
	let (outcomes, _) = Run { bugs: Bugs::all(), ..Run::default() }.checks().await;
	assert_eq!(status(&outcomes, "ERR-01"), Status::Warn);
	assert_eq!(status(&outcomes, "RESP-04"), Status::Warn);
	assert!(matches!(status(&outcomes, "SAFE-01"), Status::Fail | Status::Skip));
	assert_eq!(status(&outcomes, "PAY-03"), Status::Fail);
	for id in INVOICE_CHECKS {
		assert_eq!(status(&outcomes, id), Status::Fail, "{id}");
	}
}

#[tokio::test]
async fn each_invoice_bug_is_caught_on_its_own() {
	let none = Bugs::default();
	let cases = [
		("CORE-01", Bugs { wrong_amount: true, ..none }),
		("CORE-02", Bugs { ignore_description_hash: true, ..none }),
		("CORE-03", Bugs { lookup_by_hash_only: true, ..none }),
		("CORE-04", Bugs { never_settles: true, ..none }),
		("CORE-05", Bugs { unknown_invoice_error: true, ..none }),
	];
	for (id, bugs) in cases {
		let (outcomes, _) = Run { bugs, ..Run::default() }.checks().await;
		let failed: Vec<&str> = outcomes
			.iter()
			.filter(|outcome| outcome.is_problem())
			.map(|outcome| outcome.id)
			.collect();
		assert_eq!(failed, [id]);
	}
}
