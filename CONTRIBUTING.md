# Contributing

## Layout

```
src/
├── main.rs        Entry point: runs the stages and sets the exit code
├── cli.rs         Command-line arguments
├── config.rs      nwc-check.toml: loading, merging with arguments, template
├── polar.rs       Finds an LND node's URL, macaroon and cert in Polar
├── ui.rs          Terminal output: progress, sections, problems, summary
├── lib.rs
├── wallet.rs      Sends raw NIP-47 requests and decrypts replies
├── check.rs       Check, Level, Severity, Spec, Status, Outcome and Summary types
├── evidence.rs    Records the requests each check sends
├── report/        JSON, HTML and JUnit reports
├── counterparty/  Pays and gets paid by the wallet (Counterparty trait, LND, Lightning address)
└── checks/
    ├── mod.rs     Context shared by all checks, sections and the runner
    ├── discovery.rs
    ├── encryption.rs
    ├── response.rs
    ├── errors.rs
    ├── safety.rs
    ├── invoices.rs
    ├── payments.rs
    ├── nwc13.rs       NWC-13 get_budget
    ├── ledger.rs      Runs on its own after the others, since it compares balances
    └── budget.rs      Runs last, since it uses up the connection's budget
tests/
├── support/       Mock wallet service, counterparty and Lightning address server
└── mock_wallet.rs Runs every check against a correct and a buggy mock wallet
examples/
├── mock_wallet.rs Runs the mock wallet so the CLI can be tried locally
└── lnurl_server.rs Serves a Lightning address backed by an LND node, for regtest
```

A run has three stages:

1. **Connect.** `Wallet` connects to the relays in the connection string. It builds every request itself rather than going through an NWC client, so checks can control the encryption tag, signer and expiration of each request.
2. **Load.** `Context` fetches the wallet's info event and the replies that several checks share, such as `get_info`, once.
3. **Check.** Each section in `checks/` turns those replies, plus any requests of its own, into an `Outcome` per check. Sections run concurrently and are reported in a fixed order as they finish. Ledger and Budget then run one at a time.

To add a check, declare it with `Check::new` in the module for its section, add it to that module's `CHECKS` list and return its outcome from the module's `run` function. Then add a mock wallet bug that the check must catch, and list the check in [docs/checks.md](docs/checks.md).

## Tests

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The tests run every check against the mock wallet service (`tests/support`), once behaving correctly and once with deliberate bugs that the checks must catch.
