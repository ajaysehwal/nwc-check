mod support;

use std::time::Duration;

use nostr_sdk::prelude::MockRelay;
use nwc_check::check::{Outcome, Status};
use nwc_check::checks::{self, Context, Payments};
use nwc_check::wallet::Wallet;

use crate::support::{Bugs, MockCounterparty, MockNetwork, MockWallet};

async fn run_checks(bugs: Bugs, with_counterparty: bool) -> Vec<Outcome> {
	let relay = MockRelay::run().await.unwrap();
	let network = MockNetwork::default();
	let uri = MockWallet::start(&relay, bugs, network.clone()).await;
	let wallet = Wallet::connect(uri, Duration::from_secs(3)).await.unwrap();
	let mut ctx = Context::load(wallet, Some("regtest".to_owned())).await.unwrap();
	if with_counterparty {
		let node = Box::new(MockCounterparty(network));
		ctx = ctx.with_payments(Payments { node, amount_msat: 10_000 });
	}
	checks::run(&ctx).await
}

fn status(outcomes: &[Outcome], id: &str) -> Status {
	outcomes.iter().find(|outcome| outcome.id == id).map(|outcome| outcome.status).unwrap()
}

#[tokio::test]
async fn compliant_wallet_has_no_failures() {
	let outcomes = run_checks(Bugs::default(), true).await;
	let problems: Vec<&Outcome> = outcomes
		.iter()
		.filter(|outcome| matches!(outcome.status, Status::Fail | Status::Warn))
		.collect();
	assert!(problems.is_empty(), "{problems:#?}");
	assert_eq!(outcomes.len(), checks::catalog().len());
	for id in ["PAY-01", "PAY-02", "PAY-03"] {
		assert_eq!(status(&outcomes, id), Status::Pass, "{id}");
	}
}

#[tokio::test]
async fn payment_checks_skip_without_counterparty() {
	let outcomes = run_checks(Bugs::default(), false).await;
	for id in ["PAY-01", "PAY-02", "PAY-03"] {
		assert_eq!(status(&outcomes, id), Status::Skip, "{id}");
	}
}

#[tokio::test]
async fn buggy_wallet_is_caught() {
	let bugs = Bugs {
		answer_expired: true,
		wrong_error_code: true,
		skip_p_tag: true,
		wrong_preimage: true,
	};
	let outcomes = run_checks(bugs, true).await;
	assert_eq!(status(&outcomes, "ERR-01"), Status::Warn);
	assert_eq!(status(&outcomes, "RESP-04"), Status::Warn);
	assert!(matches!(status(&outcomes, "SAFE-01"), Status::Fail | Status::Skip));
	assert_eq!(status(&outcomes, "PAY-03"), Status::Fail);
}
