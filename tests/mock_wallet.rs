mod support;

use std::time::Duration;

use nostr_sdk::prelude::MockRelay;
use nwc_check::check::{Outcome, Status};
use nwc_check::checks::{self, Context};
use nwc_check::wallet::Wallet;

use crate::support::{Bugs, MockWallet};

async fn run_checks(bugs: Bugs) -> Vec<Outcome> {
	let relay = MockRelay::run().await.unwrap();
	let uri = MockWallet::start(&relay, bugs).await;
	let wallet = Wallet::connect(uri, Duration::from_secs(3)).await.unwrap();
	let ctx = Context::load(wallet, Some("regtest".to_owned())).await.unwrap();
	checks::run(&ctx).await
}

fn status(outcomes: &[Outcome], id: &str) -> Status {
	outcomes.iter().find(|outcome| outcome.id == id).map(|outcome| outcome.status).unwrap()
}

#[tokio::test]
async fn compliant_wallet_has_no_failures() {
	let outcomes = run_checks(Bugs::default()).await;
	let problems: Vec<&Outcome> = outcomes
		.iter()
		.filter(|outcome| matches!(outcome.status, Status::Fail | Status::Warn))
		.collect();
	assert!(problems.is_empty(), "{problems:#?}");
	assert_eq!(outcomes.len(), checks::catalog().len());
}

#[tokio::test]
async fn buggy_wallet_is_caught() {
	let bugs = Bugs { answer_expired: true, wrong_error_code: true, skip_p_tag: true };
	let outcomes = run_checks(bugs).await;
	assert_eq!(status(&outcomes, "ERR-01"), Status::Warn);
	assert_eq!(status(&outcomes, "RESP-04"), Status::Warn);
	assert!(matches!(status(&outcomes, "SAFE-01"), Status::Fail | Status::Skip));
}
