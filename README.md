# nwc-check

Conformance and safety checks for [Nostr Wallet Connect](https://github.com/nostr-protocol/nips/blob/master/47.md) (NIP-47) wallet services.

`nwc-check` connects to a wallet service the way an app would, sends it real NIP-47 requests, including malformed and adversarial ones, and reports whether it follows the spec and handles requests safely.

All current checks are read-only and never spend funds.

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
| `--network <NETWORK>` | Fail if the wallet reports a different network (`mainnet`, `testnet`, `signet`, `regtest`) |
| `--only <IDS>` | Only report checks whose ID starts with one of these prefixes, e.g. `DISC,ENC-01` |
| `--format <FORMAT>` | `text` (default) or `json` |
| `--timeout <SECS>` | Seconds to wait for each reply (default 10) |
| `--strict` | Treat warnings as failures |
| `--no-color` | Disable colored output (`NO_COLOR` is also respected) |

`nwc-check list` prints all checks.

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

Checks for methods or encryption schemes the wallet doesn't advertise are skipped. SAFE-01 is skipped when the relays drop expired events themselves (NIP-40), since the wallet never sees the request.

## Development

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The tests run every check against a mock wallet service on a local relay, once behaving correctly and once with deliberate bugs that the checks must catch.

## License

MIT
