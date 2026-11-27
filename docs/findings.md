# Findings

Bugs nwc-check has found in real wallet services and NWC libraries, and where each fix stands. Each one was reported upstream before it was listed here.

| Project | Check | Severity | Problem | Status |
|---|---|---|---|---|
| ZEUS | CORE-01 | HIGH | `make_invoice` returned a `payment_hash` that didn't match the invoice | Fixed in [ZeusLN/zeus#5869](https://github.com/ZeusLN/zeus/pull/5869) |
| ZEUS | CORE-02 | MEDIUM | `make_invoice` ignored `description_hash` | Refused in [ZeusLN/zeus#5870](https://github.com/ZeusLN/zeus/pull/5870), supported in [ZeusLN/zeus#5901](https://github.com/ZeusLN/zeus/pull/5901) (open) |
| Alby JS SDK | ENC-03 | LOW | Unknown encryption scheme decrypted as NIP-44 instead of refused | [getAlby/js-sdk#602](https://github.com/getAlby/js-sdk/pull/602) (open) |
| Alby Hub | SAFE-01 | HIGH | Expired requests are still processed | [getAlby/hub#2750](https://github.com/getAlby/hub/issues/2750) (open) |

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

## Adding a finding

Report the bug to the project first, and wait until the report or fix is public. Then add a row to the table and a section with the check, the setup it was found on, what went wrong, why it matters to users, and the link. Keep the severity the check has in [checks.md](checks.md).
