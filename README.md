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
| `--readonly-uri <URI>` | Second connection without `pay_invoice`, for SAFE-05 (or `NWC_READONLY_URI`) |
| `--output <PATH>`, `-o` | Write the report to this file. Defaults to `nwc-check-report.html` for `html`, and to stdout for `json` and `junit` |
| `--config <PATH>` | Settings file (default `./nwc-check.toml` when it exists) |
| `--network <NETWORK>` | Fail if the wallet reports a different network (`mainnet`, `testnet`, `signet`, `regtest`) |
| `--only <IDS>` | Only report checks whose ID starts with one of these prefixes, e.g. `DISC,ENC-01` |
| `--format <FORMAT>` | `text` (default), `json`, `html` or `junit` |
| `--timeout <SECS>` | Seconds to wait for each reply (default 10) |
| `--strict` | Treat warnings as failures |
| `--no-color` | Disable colored output (`NO_COLOR` is also respected) |

`nwc-check list` prints all checks.

To share a run, also write it as a single HTML page:

```sh
nwc-check run --format html "nostr+walletconnect://..."
```

The run shows its usual progress and results, writes `nwc-check-report.html` (or the file given with `--output`), and ends with a `file://` link to open it. The page has no scripts and loads nothing from the network. It shows the verdict, each problem with its evidence and a link to the spec, and every result grouped by section.

For CI, `--format junit` writes JUnit XML, to stdout or to the file given with `--output`, with one test suite per section and one test case per check. Failures carry the reason, the spec link and the requests. Skips are marked as skipped. Warnings fail only with `--strict`, matching the exit code.

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
| `--budget-sats <SATS>` | Remaining budget of the connection, for wallets without `get_budget` |

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

nwc-check only tests what NIP-47 and the published NWC extensions define, plus safety rules that apply to every wallet. It doesn't test one wallet's own methods or features. Check IDs are never reused: SAFE-07 was removed for testing Alby Hub's `create_connection`, which no spec defines.

Each check has a level: `MUST` and `SAFETY` problems are failures, `SHOULD` problems are warnings. `SAFETY` checks cover behavior the spec doesn't require but that protects funds.

Each check also has a severity: how much harm a problem can do to the user, which tells you what to fix first. The level decides whether a problem fails or warns; the severity doesn't change that.

| Severity | Meaning |
|---|---|
| `CRITICAL` | Funds can be lost beyond what the user allowed |
| `HIGH` | Money can be wrong, lost or misreported |
| `MEDIUM` | Apps can break or misread the wallet |
| `LOW` | A detail that rarely affects users |

Problems are listed most severe first, the summary counts them by severity, and every report format shows it: `severity` and `summary.problems` in JSON, a badge in HTML, and the failure `type` plus a `severity` property in JUnit.

Each check also names the spec it comes from, pinned to the commit it was written against: NIP-47 [`c53877571f96`](https://github.com/nostr-protocol/nips/blob/c53877571f96/47.md), NIP-40 [`f5a15ea27e06`](https://github.com/nostr-protocol/nips/blob/f5a15ea27e06/40.md) or NWC-13 [`36ab132bbce1`](https://github.com/nostr-wallet-connect/nwc/blob/36ab132bbce1/13.md). SAFE-02, SAFE-03, SAFE-04 and SAFE-06 have none. The spec appears in `nwc-check list`, in the JSON `spec` field, and under each problem in the text output.

The JSON report also records evidence for each check: every request it sent, as `method`, `request` and `reply` event IDs and `ms`, plus the check's total `duration_ms`. Use the event IDs to find the exact events on the relay. Checks built on the shared `get_info` reply cite that exchange. Checks that send nothing, such as DISC-01, have none.

| ID | Severity | Level | Check |
|---|---|---|---|
| DISC-01 | MEDIUM | MUST | Info event is published |
| DISC-02 | MEDIUM | MUST | Info event lists methods |
| DISC-03 | LOW | SHOULD | NIP-44 encryption is advertised |
| DISC-04 | MEDIUM | SHOULD | get_info methods match the info event |
| DISC-05 | HIGH | MUST | get_info reports a valid network |
| DISC-06 | LOW | SHOULD | Extensions match between info event and get_info |
| ENC-01 | MEDIUM | MUST | NIP-44 request gets a NIP-44 reply |
| ENC-02 | MEDIUM | MUST | NIP-04 request gets a NIP-04 reply |
| ENC-03 | LOW | MUST | Unsupported scheme returns UNSUPPORTED_ENCRYPTION |
| ENC-04 | MEDIUM | MUST | Untagged request is treated as NIP-04 |
| RESP-01 | MEDIUM | MUST | result_type matches the method |
| RESP-02 | MEDIUM | MUST | Successful reply has a null error |
| RESP-03 | LOW | MUST | Error has a code and a message |
| RESP-04 | MEDIUM | SHOULD | Reply references the request and client |
| RESP-05 | MEDIUM | MUST | get_balance returns a balance in msats |
| ERR-01 | LOW | SHOULD | Unknown method returns NOT_IMPLEMENTED |
| ERR-02 | LOW | SHOULD | Unknown client returns UNAUTHORIZED |
| ERR-03 | MEDIUM | SAFETY | Malformed params return an error |
| SAFE-01 | HIGH | SAFETY | Expired requests are ignored |
| SAFE-03 | CRITICAL | SAFETY | A duplicated pay_invoice is paid once |
| SAFE-02 | CRITICAL | SAFETY | Concurrent payments stay within the budget |
| SAFE-04 | CRITICAL | SAFETY | amount can't change a fixed-amount invoice |
| SAFE-05 | CRITICAL | SAFETY | A read-only connection can't pay |
| SAFE-06 | HIGH | SAFETY | A stale pay_invoice is not paid |
| EXT13-01 | LOW | MUST | get_budget is listed in the info event and get_info |
| EXT13-02 | MEDIUM | MUST | get_budget is empty or has every required field |
| EXT13-03 | LOW | MUST | A renewing budget has renews_at, a never budget doesn't |
| EXT13-04 | HIGH | SAFETY | used_budget rises by the amount plus fees_paid |
| CORE-01 | HIGH | MUST | make_invoice matches amount, description and expiry |
| CORE-02 | MEDIUM | MUST | make_invoice sets the description_hash |
| CORE-03 | MEDIUM | MUST | lookup_invoice finds by payment_hash and invoice |
| CORE-04 | HIGH | MUST | lookup_invoice shows pending, then settled |
| CORE-05 | LOW | MUST | Unknown invoice returns NOT_FOUND |
| PAY-01 | HIGH | MUST | make_invoice returns a payable invoice |
| PAY-02 | HIGH | MUST | pay_invoice delivers the payment |
| PAY-03 | HIGH | SAFETY | pay_invoice returns the matching preimage |
| CORE-07 | LOW | SHOULD | pay_invoice reports plausible fees_paid |
| CORE-08 | CRITICAL | MUST | pay_invoice pays the amount given for an amountless invoice |
| CORE-09 | HIGH | SAFETY | Balance drops by the amount plus fees_paid |

Checks for methods or encryption schemes the wallet doesn't advertise are skipped. A reply without an `e` tag can't be matched to its request; it's reported as such, and RESP-04 warns, rather than looking like no reply. SAFE-01 is skipped when the relays drop expired events themselves (NIP-40), since the wallet never sees the request. Payment checks, CORE-04, CORE-07 to CORE-09, SAFE-02 to SAFE-06 and EXT13-04, which move funds, are skipped without a counterparty and `--allow-spend`, and on mainnet. Invoice checks decode the returned BOLT11 invoice rather than trusting the reply's fields.

SAFE-03 sends one signed `pay_invoice` event, then publishes the identical event again after the wallet replies, and counts settled payments for that hash with `list_transactions`. It reserves twice the payment amount from `--max-sats`, since a wallet with this bug pays twice. It's skipped when the relays don't pass the copy on.

CORE-09 and EXT13-04 share one payment: CORE-09 compares `get_balance`, and EXT13-04 compares `used_budget` from [NWC-13](https://github.com/nostr-wallet-connect/nwc/blob/main/13.md) `get_budget`, before and after it. EXT13 checks are skipped when the wallet doesn't advertise `get_budget`. It allows up to 1 sat of difference for wallets that round to whole sats. Payment checks are skipped, not failed, when the wallet refuses with `QUOTA_EXCEEDED`, since that means the connection's budget ran out.

SAFE-04 pays a fixed-amount invoice with `amount` set to twice the invoice amount. It fails if the counterparty receives anything other than the invoice amount, and passes if the wallet rejects the request or ignores the param.

SAFE-06 sends a `pay_invoice` request signed an hour ago, with no `expiration` tag, for a counterparty invoice. It fails if the counterparty receives the payment: a relay, or anyone holding a captured request, can send it again, and a wallet with no age limit pays it again. NIP-47 only defines the `expiration` tag, so this is a safety rule no spec requires, like SAFE-03. It passes when the wallet ignores or rejects the request, and it's skipped when the relays drop the old event themselves.

SAFE-05 needs a second connection to the same wallet that doesn't have `pay_invoice` permission. Pass it with `--readonly-uri` or `NWC_READONLY_URI`; like the main connection string, it's never read from the settings file. The read-only connection tries to pay a counterparty invoice, and the check passes only if the wallet replies `RESTRICTED` and nothing arrives.

SAFE-02 reads the connection's remaining budget with `get_budget`, or from `--budget-sats` for wallets without it. It then sends five payments at once, sized so four fit the budget and five don't, and checks how much the counterparty actually received. It runs after every other check because it uses up the connection's budget, so use a connection with a small budget. The five payments together must fit under `--max-sats`.

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
├── check.rs       Check, Level, Severity, Spec, Status, Outcome and Summary types
├── evidence.rs    Records the requests each check sends
├── report/        JSON, HTML and JUnit reports
├── counterparty/  Node that pays and gets paid by the wallet (Counterparty trait, LND client)
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
├── support/       Mock wallet service and counterparty on a local relay
└── mock_wallet.rs Runs every check against a correct and a buggy mock wallet
examples/
└── mock_wallet.rs Runs the mock wallet so the CLI can be tried locally
```

A run has three stages:

1. **Connect.** `Wallet` connects to the relays in the connection string. It builds every request itself rather than going through an NWC client, so checks can control the encryption tag, signer and expiration of each request.
2. **Load.** `Context` fetches the wallet's info event and the replies that several checks share, such as `get_info`, once.
3. **Check.** Each section in `checks/` turns those replies, plus any requests of its own, into an `Outcome` per check. Sections run concurrently and are reported in a fixed order as they finish. Ledger and Budget then run one at a time.

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
