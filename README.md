# nwc-check

Conformance and safety checks for [Nostr Wallet Connect](https://github.com/nostr-protocol/nips/blob/master/47.md) (NIP-47) wallet services.

**Status:** early prototype. Only read-only checks exist so far, and they never spend funds.

## Usage

```sh
cargo run -- run "nostr+walletconnect://<wallet-pubkey>?relay=wss://...&secret=..."
```

Use a connection string from a test wallet. The connection string contains a secret, so don't paste it anywhere public.

## Checks

| ID | What it checks |
|---|---|
| DISC-01 | The wallet service publishes an info event (kind 13194) |
| DISC-02 | The info event lists the supported methods |
| DISC-03 | The info event advertises `nip44_v2` encryption |
| INFO-01 | `get_info` answers with a valid network |

The command exits with status 1 if any check fails.

## License

MIT
