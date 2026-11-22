use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;

/// One request sent to the wallet and the reply it got, if any.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Exchange {
	pub method: String,
	pub request: String,
	pub reply: Option<String>,
	pub ms: u64,
}

/// The exchanges a piece of check code made, and how long it took.
#[derive(Clone, Debug, Default)]
pub struct Evidence {
	pub exchanges: Vec<Exchange>,
	pub took: Duration,
}

tokio::task_local! {
	static RECORDER: Arc<Mutex<Vec<Exchange>>>;
}

/// Adds an exchange to the innermost `track` call, if there is one.
pub(crate) fn record(exchange: Exchange) {
	let _ = RECORDER.try_with(|exchanges| lock(exchanges).push(exchange));
}

/// Runs `future` and collects every exchange it makes, including in futures it joins.
pub async fn track<T>(future: impl Future<Output = T>) -> (T, Evidence) {
	let recorder = Arc::default();
	let started = Instant::now();
	let output = RECORDER.scope(Arc::clone(&recorder), future).await;
	let exchanges = std::mem::take(&mut *lock(&recorder));
	(output, Evidence { exchanges, took: started.elapsed() })
}

fn lock(exchanges: &Mutex<Vec<Exchange>>) -> MutexGuard<'_, Vec<Exchange>> {
	exchanges.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
	use super::*;

	fn exchange(method: &str) -> Exchange {
		Exchange { method: method.to_owned(), request: "id".to_owned(), reply: None, ms: 1 }
	}

	#[tokio::test]
	async fn concurrent_tracks_keep_their_own_exchanges() {
		let ((), outer) = track(async {
			record(exchange("outer"));
			let (((), first), ((), second)) = tokio::join!(
				track(async { record(exchange("first")) }),
				track(async { record(exchange("second")) }),
			);
			assert_eq!(first.exchanges, [exchange("first")]);
			assert_eq!(second.exchanges, [exchange("second")]);
		})
		.await;
		assert_eq!(outer.exchanges, [exchange("outer")]);
	}

	#[test]
	fn recording_outside_track_is_ignored() {
		record(exchange("nowhere"));
	}
}
