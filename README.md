# nwc-check

Conformance and safety checks for [Nostr Wallet Connect](https://github.com/nostr-protocol/nips/blob/master/47.md) (NIP-47) wallet services.

`nwc-check` connects to a wallet service the way an app would, sends it real NIP-47 requests, including malformed and adversarial ones, and reports whether it follows the spec and handles requests safely.

It has already found bugs in ZEUS, Primal, Alby Hub and the Alby JS SDK. See [docs/findings.md](docs/findings.md).

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
| `--with-flood` | Run SAFE-15, which sends a burst of 60 `get_info` requests to test rate limiting |
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

The checks are grouped in ten sections, from discovery and encryption to payments and budget safety. Each has a severity (`CRITICAL` to `LOW`) that says what to fix first, and a level: `MUST` and `SAFETY` problems fail the run, `SHOULD` problems warn.

See [docs/checks.md](docs/checks.md) for every check, what it sends and when it skips, or run `nwc-check list`.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for how the code is laid out, how to add a check and how to run the tests.

## License

MIT
