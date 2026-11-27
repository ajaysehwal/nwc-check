# Findings

Bugs nwc-check has found in real wallet services and NWC libraries, and where each fix stands. Each one was reported upstream before it was listed here.

| Project | Check | Severity | Problem | Status |
|---|---|---|---|---|
| ZEUS | CORE-01 | HIGH | `make_invoice` returned a `payment_hash` that didn't match the invoice | Fixed in [ZeusLN/zeus#5869](https://github.com/ZeusLN/zeus/pull/5869) |
| ZEUS | CORE-02 | MEDIUM | `make_invoice` ignored `description_hash` | Refused in [ZeusLN/zeus#5870](https://github.com/ZeusLN/zeus/pull/5870), supported in [ZeusLN/zeus#5901](https://github.com/ZeusLN/zeus/pull/5901) (open) |
| Alby JS SDK | ENC-03 | LOW | Unknown encryption scheme decrypted as NIP-44 instead of refused | [getAlby/js-sdk#602](https://github.com/getAlby/js-sdk/pull/602) (open) |
| Alby Hub | SAFE-01 | HIGH | Expired requests are still processed | [getAlby/hub#2750](https://github.com/getAlby/hub/issues/2750) (open) |
| Primal | SAFE-01 | HIGH | Expired requests are still processed | [PrimalHQ/primal-android-app#1123](https://github.com/PrimalHQ/primal-android-app/issues/1123) (open) |
| Primal | CORE-02 | MEDIUM | `make_invoice` drops `description_hash` but reports it in the reply | [PrimalHQ/primal-android-app#1124](https://github.com/PrimalHQ/primal-android-app/issues/1124) (open) |
| Primal | ENC-03 | LOW | Unsupported encryption scheme gets `INTERNAL` | [PrimalHQ/primal-android-app#1125](https://github.com/PrimalHQ/primal-android-app/issues/1125) (open) |

Status as of 2026-11-27.

## ZEUS: make_invoice returned the wrong payment_hash

- **Check:** [CORE-01](checks.md#invoices), make_invoice matches amount, description and expiry
- **Setup:** ZEUS on LND (REST), Polar regtest
- **Fix:** [ZeusLN/zeus#5869](https://github.com/ZeusLN/zeus/pull/5869), merged 2026-11-23

The reply's `payment_hash` was LND's `r_hash`, which LND REST returns as base64 and ZEUS made URL-safe. It didn't match the hex payment hash inside the invoice. An app that tracks the payment by `payment_hash` from the reply never finds it. The reply now uses the hash decoded from the invoice, as the stored activity row already did.

## ZEUS: make_invoice ignored description_hash

- **Check:** [CORE-02](checks.md#invoices), make_invoice sets the description_hash
- **Setup:** ZEUS on LND (REST), Polar regtest
- **Fix:** [ZeusLN/zeus#5870](https://github.com/ZeusLN/zeus/pull/5870), merged 2026-11-23; [ZeusLN/zeus#5901](https://github.com/ZeusLN/zeus/pull/5901), open

A request with a `description_hash` got an invoice with a plain description. LNURL-pay and Nostr zaps (NIP-57) need the hash in the invoice so the payer's wallet can verify it, so those invoices couldn't be verified. #5870 answers such requests with `NOT_IMPLEMENTED` instead of returning a wrong invoice. #5901 adds real support on the backends that can set the hash: LND, embedded LND, LNC, CLN REST and LDK Node.

## Alby JS SDK: unknown encryption scheme treated as NIP-44

- **Check:** [ENC-03](checks.md#encryption), unsupported scheme returns UNSUPPORTED_ENCRYPTION
- **Setup:** a ZEUS wallet, which uses the SDK's `NWCWalletService`
- **Fix:** [getAlby/js-sdk#602](https://github.com/getAlby/js-sdk/pull/602), open

`NWCWalletService` only compared the `encryption` tag with `nip04`, so any other value, including a scheme it doesn't support, was decrypted as `nip44_v2`, handled and answered. NIP-47 says the wallet replies `UNSUPPORTED_ENCRYPTION`. Every wallet built on the SDK inherits this, not just the one it was found in.

## Alby Hub: expired requests are still processed

- **Check:** [SAFE-01](checks.md#safety), expired requests are ignored
- **Setup:** Alby Hub through `relay.getalby.com`, which passes expired events on
- **Fix:** [getAlby/hub#2750](https://github.com/getAlby/hub/issues/2750), open

NIP-47 says a request received after its `expiration` tag "should be ignored". Hub never reads the tag, so it answers expired requests. Every method goes through the same handler, so an expired `pay_invoice`, for example one delivered late after a relay outage, would be paid long after the app gave up on it. The issue includes a suggested fix.

## Primal: expired requests are still processed

- **Check:** [SAFE-01](checks.md#safety), expired requests are ignored
- **Setup:** Primal iOS 3.5.65. Its NWC service is shared Kotlin code from primal-android-app, so Android is affected too.
- **Fix:** [PrimalHQ/primal-android-app#1123](https://github.com/PrimalHQ/primal-android-app/issues/1123), open

The service never reads the `expiration` tag. The service only runs while the app is open, so a request sent while it's closed waits on the relay and is answered when the app next opens, possibly hours after the app or user gave up on it. An expired `pay_invoice` would be paid the same way.

## Primal: make_invoice drops description_hash

- **Check:** [CORE-02](checks.md#invoices), make_invoice sets the description_hash
- **Setup:** Primal iOS 3.5.65
- **Fix:** [PrimalHQ/primal-android-app#1124](https://github.com/PrimalHQ/primal-android-app/issues/1124), open

The `description_hash` never reaches the backend, so the invoice gets a plain description. The reply still returns the requested hash, so the client is told the invoice is bound to its request when it isn't. LNURL-pay and zap payers that check the hash refuse these invoices.

## Primal: unsupported encryption scheme gets INTERNAL

- **Check:** [ENC-03](checks.md#encryption), unsupported scheme returns UNSUPPORTED_ENCRYPTION
- **Setup:** Primal iOS 3.5.65
- **Fix:** [PrimalHQ/primal-android-app#1125](https://github.com/PrimalHQ/primal-android-app/issues/1125), open

An unknown `encryption` value is treated as `nip04`, decryption fails, and the reply is `INTERNAL`. Clients can't tell this from a server fault, so they don't fall back to a supported scheme.

## Adding a finding

Report the bug to the project first, and wait until the report or fix is public. Then add a row to the table and a section with the check, the setup it was found on, what went wrong, why it matters to users, and the link. Keep the severity the check has in [checks.md](checks.md).
