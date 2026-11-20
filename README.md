# nwc-check

Conformance and safety checks for [Nostr Wallet Connect](https://github.com/nostr-protocol/nips/blob/master/47.md) (NIP-47) wallet services.

`nwc-check` connects to a wallet service the way an app would, sends it real NIP-47 requests, including malformed and adversarial ones, and reports whether it follows the spec and handles requests safely.

By default no check moves funds. Some create unpaid invoices in the wallet. Payment checks only run with a counterparty node and `--allow-spend`, and never on mainnet.

## Install

```sh
cargo install --git https://github.com/ajaysehwal/nwc-check
```

## Usage

```sh
nwc-check run "nostr+walletconnect://<wallet-pubkey>?relay=wss://...&secret=..."
```

The connection string contains a secret. To keep it out of your shell history, pass it through `NWC_URI` instead:

```sh
export NWC_URI="nostr+walletconnect://..."
nwc-check run
```

| Option | Description |
|---|---|
| `--config <PATH>` | Settings file (default `./nwc-check.toml` when it exists) |
| `--network <NETWORK>` | Fail if the wallet reports a different network (`mainnet`, `testnet`, `signet`, `regtest`) |
| `--only <IDS>` | Only report checks whose ID starts with one of these prefixes, e.g. `DISC,ENC-01` |
| `--format <FORMAT>` | `text` (default) or `json` |
| `--timeout <SECS>` | Seconds to wait for each reply (default 10) |
| `--strict` | Treat warnings as failures |
| `--no-color` | Disable colored output (`NO_COLOR` is also respected) |

`nwc-check list` prints all checks.

### Settings file

Instead of passing options every time, keep them in `nwc-check.toml`:

```sh
nwc-check init                # write nwc-check.toml with every setting commented
nwc-check init --polar alice  # fill in the counterparty from an LND node in Polar
nwc-check run "nostr+walletconnect://..."
```

`run` reads `./nwc-check.toml` when it exists, or the file given by `--config` (or `NWC_CHECK_CONFIG`). Options on the command line override the file. The connection string is never stored in it.

```toml
timeout = 10
network = "regtest"
strict = false

[lnd]
url = "https://127.0.0.1:8082"
macaroon = "~/.polar/networks/1/volumes/lnd/alice/data/chain/bitcoin/regtest/admin.macaroon"
cert = "~/.polar/networks/1/volumes/lnd/alice/tls.cert"

[spending]
allow = true
amount = 10
max_sats = 100
```

Paths can be relative to the file or start with `~/`. Unknown keys are rejected, so a typo doesn't silently fall back to a default.

### Payment checks

Payment checks need a second node you control, the counterparty. It pays an invoice from `make_invoice` and creates an invoice for the wallet to pay with `pay_invoice`. Only LND is supported for now.

```sh
nwc-check run --allow-spend \
  --lnd-url https://localhost:8080 \
  --lnd-macaroon ~/.lnd/data/chain/bitcoin/regtest/admin.macaroon \
  --lnd-cert ~/.lnd/tls.cert
```

The counterparty needs a route to the wallet, for example a channel on regtest or signet.

Spending is guarded:

- Without `--allow-spend`, no request that can move funds is sent, and the counterparty isn't contacted.
- If the wallet or the counterparty is on mainnet, or they're on different networks, payment checks are skipped. There is no override.
- Each payment is 10 sats by default (`--amount`), and all payments in a run together never move more than `--max-sats` (default 100). A payment that would go over is skipped.

| Option | Description |
|---|---|
| `--lnd-url <URL>` | LND REST URL |
| `--lnd-macaroon <PATH>` | Macaroon with info:read, invoices:write and offchain:write, e.g. `admin.macaroon` |
| `--lnd-cert <PATH>` | TLS certificate, for nodes with a self-signed cert |
| `--allow-spend` | Allow checks that move funds |
| `--amount <SATS>` | Amount of each test payment (default 10) |
| `--max-sats <SATS>` | Most sats a run may move in total (default 100) |

### Output

Progress is shown live while the checks run. Each section is printed as soon as it completes, and every problem is explained at the end:

```
Responses
  ✔ RESP-01  result_type matches the method                     get_info
  ⚠ RESP-04  Reply references the request and client            no p tag for the client

Problems found (1)

  ⚠ RESP-04 Reply references the request and client  SHOULD
    Found  no p tag for the client
    Why    Without e and p tags, clients can't match replies to their requests.

⚠ Passed with warnings  15 passed · 1 warning · 0 failed · 3 skipped  (42 ms)
```

### Try it without a wallet

The repository includes a mock wallet service that runs on a local relay and prints its connection string:

```sh
cargo run --example mock_wallet            # a wallet that behaves correctly
cargo run --example mock_wallet -- --buggy # a wallet with deliberate bugs
```

Then pass the printed connection string to `nwc-check run` in another terminal.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | No failures |
| 1 | At least one check failed, or a warning with `--strict` |
| 2 | Invalid input or the wallet could not be reached |

## Checks

Each check has a level: `MUST` and `SAFETY` problems are failures, `SHOULD` problems are warnings. `SAFETY` checks cover behavior the spec doesn't require but that protects funds.

| ID | Level | Check |
|---|---|---|
| DISC-01 | MUST | Info event is published |
| DISC-02 | MUST | Info event lists methods |
| DISC-03 | SHOULD | NIP-44 encryption is advertised |
| DISC-04 | SHOULD | get_info methods match the info event |
| DISC-05 | MUST | get_info reports a valid network |
| DISC-06 | SHOULD | Extensions match between info event and get_info |
| ENC-01 | MUST | NIP-44 request gets a NIP-44 reply |
| ENC-02 | MUST | NIP-04 request gets a NIP-04 reply |
| ENC-03 | MUST | Unsupported scheme returns UNSUPPORTED_ENCRYPTION |
| ENC-04 | MUST | Untagged request is treated as NIP-04 |
| RESP-01 | MUST | result_type matches the method |
| RESP-02 | MUST | Successful reply has a null error |
| RESP-03 | MUST | Error has a code and a message |
| RESP-04 | SHOULD | Reply references the request and client |
| RESP-05 | MUST | get_balance returns a balance in msats |
| ERR-01 | SHOULD | Unknown method returns NOT_IMPLEMENTED |
| ERR-02 | SHOULD | Unknown client returns UNAUTHORIZED |
| ERR-03 | SAFETY | Malformed params return an error |
| SAFE-01 | SAFETY | Expired requests are ignored |
| SAFE-03 | SAFETY | A duplicated pay_invoice is paid once |
| CORE-01 | MUST | make_invoice matches amount, description and expiry |
| CORE-02 | MUST | make_invoice sets the description_hash |
| CORE-03 | MUST | lookup_invoice finds by payment_hash and invoice |
| CORE-04 | MUST | lookup_invoice shows pending, then settled |
| CORE-05 | MUST | Unknown invoice returns NOT_FOUND |
| PAY-01 | MUST | make_invoice returns a payable invoice |
| PAY-02 | MUST | pay_invoice delivers the payment |
| PAY-03 | SAFETY | pay_invoice returns the matching preimage |

Checks for methods or encryption schemes the wallet doesn't advertise are skipped. SAFE-01 is skipped when the relays drop expired events themselves (NIP-40), since the wallet never sees the request. Payment checks, CORE-04 and SAFE-03, which move funds, are skipped without a counterparty and `--allow-spend`, and on mainnet. Invoice checks decode the returned BOLT11 invoice rather than trusting the reply's fields.

SAFE-03 sends one signed `pay_invoice` event, then publishes the identical event again after the wallet replies, and counts settled payments for that hash with `list_transactions`. It reserves twice the payment amount from `--max-sats`, since a wallet with this bug pays twice. It's skipped when the relays don't pass the copy on.

## Architecture

```
src/
├── main.rs        Entry point: runs the stages and sets the exit code
├── cli.rs         Command-line arguments
├── config.rs      nwc-check.toml: loading, merging with arguments, template
├── polar.rs       Finds an LND node's URL, macaroon and cert in Polar
├── ui.rs          Terminal output: progress, sections, problems, summary
├── lib.rs
├── wallet.rs      Sends raw NIP-47 requests and decrypts replies
├── check.rs       Check, Level, Status, Outcome and Summary types
├── report.rs      JSON report
├── counterparty/  Node that pays and gets paid by the wallet (Counterparty trait, LND client)
└── checks/
    ├── mod.rs     Context shared by all checks, sections and the runner
    ├── discovery.rs
    ├── encryption.rs
    ├── response.rs
    ├── errors.rs
    ├── safety.rs
    ├── invoices.rs
    └── payments.rs
tests/
├── support/       Mock wallet service and counterparty on a local relay
└── mock_wallet.rs Runs every check against a correct and a buggy mock wallet
examples/
└── mock_wallet.rs Runs the mock wallet so the CLI can be tried locally
```

A run has three stages:

1. **Connect.** `Wallet` connects to the relays in the connection string. It builds every request itself rather than going through an NWC client, so checks can control the encryption tag, signer and expiration of each request.
2. **Load.** `Context` fetches the wallet's info event and the replies that several checks share, such as `get_info`, once.
3. **Check.** Each section in `checks/` turns those replies, plus any requests of its own, into an `Outcome` per check. Sections run concurrently and are reported in a fixed order as they finish.

To add a check, declare it with `Check::new` in the module for its section, add it to that module's `CHECKS` list and return its outcome from the module's `run` function. Then add a mock wallet bug that the check must catch.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The tests run every check against the mock wallet service (`tests/support`), once behaving correctly and once with deliberate bugs that the checks must catch.

## License

MIT
