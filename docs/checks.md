# Checks

Every check nwc-check runs, grouped by the section it's reported in. `nwc-check list` prints the same list.

- [How results are graded](#how-results-are-graded)
- [When checks skip](#when-checks-skip)
- [Discovery](#discovery) · [Encryption](#encryption) · [Responses](#responses) · [Errors](#errors) · [Safety](#safety) · [Invoices](#invoices) · [Payments](#payments) · [Connection budget (NWC-13)](#connection-budget-nwc-13) · [Ledger](#ledger) · [Budget](#budget)

## How results are graded

Each check has a level. `MUST` and `SAFETY` problems are failures, `SHOULD` problems are warnings. `SAFETY` checks cover behavior the spec doesn't require but that protects funds.

Each check also has a severity: how much harm a problem can do to the user, which tells you what to fix first. The level decides whether a problem fails or warns; the severity doesn't change that.

| Severity | Meaning |
|---|---|
| `CRITICAL` | Funds can be lost beyond what the user allowed |
| `HIGH` | Money can be wrong, lost or misreported |
| `MEDIUM` | Apps can break or misread the wallet |
| `LOW` | A detail that rarely affects users |

Problems are listed most severe first, the summary counts them by severity, and every report format shows it: `severity` and `summary.problems` in JSON, a badge in HTML, and the failure `type` plus a `severity` property in JUnit.

Each check names the spec it comes from, pinned to the commit it was written against: NIP-47 [`c53877571f96`](https://github.com/nostr-protocol/nips/blob/c53877571f96/47.md), NIP-40 [`f5a15ea27e06`](https://github.com/nostr-protocol/nips/blob/f5a15ea27e06/40.md) or NWC-13 [`36ab132bbce1`](https://github.com/nostr-wallet-connect/nwc/blob/36ab132bbce1/13.md). SAFE-02, SAFE-03, SAFE-04 and SAFE-06 have none. The spec appears in `nwc-check list`, in the JSON `spec` field, and under each problem in the text output.

The JSON report records evidence for each check: every request it sent, as `method`, `request` and `reply` event IDs and `ms`, plus the check's total `duration_ms`. Use the event IDs to find the exact events on the relay. Checks built on the shared `get_info` reply cite that exchange. Checks that send nothing, such as DISC-01, have none.

## When checks skip

- Checks for methods or encryption schemes the wallet doesn't advertise are skipped.
- Checks that move funds are skipped without a counterparty and `--allow-spend`, and on mainnet: everything under Payments, Ledger and Budget, plus CORE-04 and SAFE-03 to SAFE-06.
- Payment checks are skipped, not failed, when the wallet refuses with `QUOTA_EXCEEDED`, since that means the connection's budget ran out.
- With a Lightning address as the counterparty, some checks can't run. See [below](#lightning-address-counterparty).
- A reply without an `e` tag can't be matched to its request. It's reported as such, and RESP-04 warns, rather than looking like no reply.

## Discovery

| ID | Severity | Level | Check |
|---|---|---|---|
| DISC-01 | MEDIUM | MUST | Info event is published |
| DISC-02 | MEDIUM | MUST | Info event lists methods |
| DISC-03 | LOW | SHOULD | NIP-44 encryption is advertised |
| DISC-04 | MEDIUM | SHOULD | get_info methods match the info event |
| DISC-05 | HIGH | MUST | get_info reports a valid network |
| DISC-06 | LOW | SHOULD | Extensions match between info event and get_info |

## Encryption

| ID | Severity | Level | Check |
|---|---|---|---|
| ENC-01 | MEDIUM | MUST | NIP-44 request gets a NIP-44 reply |
| ENC-02 | MEDIUM | MUST | NIP-04 request gets a NIP-04 reply |
| ENC-03 | LOW | MUST | Unsupported scheme returns UNSUPPORTED_ENCRYPTION |
| ENC-04 | MEDIUM | MUST | Untagged request is treated as NIP-04 |

## Responses

| ID | Severity | Level | Check |
|---|---|---|---|
| RESP-01 | MEDIUM | MUST | result_type matches the method |
| RESP-02 | MEDIUM | MUST | Successful reply has a null error |
| RESP-03 | LOW | MUST | Error has a code and a message |
| RESP-04 | MEDIUM | SHOULD | Reply references the request and client |
| RESP-05 | MEDIUM | MUST | get_balance returns a balance in msats |

## Errors

| ID | Severity | Level | Check |
|---|---|---|---|
| ERR-01 | LOW | SHOULD | Unknown method returns NOT_IMPLEMENTED |
| ERR-02 | LOW | SHOULD | Unknown client returns UNAUTHORIZED |
| ERR-03 | MEDIUM | SAFETY | Malformed params return an error |

## Safety

| ID | Severity | Level | Check |
|---|---|---|---|
| SAFE-01 | HIGH | SAFETY | Expired requests are ignored |
| SAFE-03 | CRITICAL | SAFETY | A duplicated pay_invoice is paid once |
| SAFE-04 | CRITICAL | SAFETY | amount can't change a fixed-amount invoice |
| SAFE-06 | HIGH | SAFETY | A stale pay_invoice is not paid |
| SAFE-05 | CRITICAL | SAFETY | A read-only connection can't pay |
| SAFE-07 | CRITICAL | SAFETY | create_connection can't grant more than its creator has |

**SAFE-01** is skipped when the relays drop expired events themselves (NIP-40), since the wallet never sees the request.

**SAFE-03** sends one signed `pay_invoice` event, then publishes the identical event again after the wallet replies, and counts settled payments for that hash with `list_transactions`. It reserves twice the payment amount from `--max-sats`, since a wallet with this bug pays twice. It's skipped when the relays don't pass the copy on.

**SAFE-04** pays a fixed-amount invoice with `amount` set to twice the invoice amount. It fails if the counterparty receives anything other than the invoice amount, and passes if the wallet rejects the request or ignores the param.

**SAFE-05** needs a second connection to the same wallet that doesn't have `pay_invoice` permission. Pass it with `--readonly-uri` or `NWC_READONLY_URI`; like the main connection string, it's never read from the settings file. The read-only connection tries to pay a counterparty invoice, and the check passes only if the wallet replies `RESTRICTED` and nothing arrives.

**SAFE-06** sends a `pay_invoice` request signed an hour ago, with no `expiration` tag, for a counterparty invoice. It fails if the counterparty receives the payment: a relay, or anyone holding a captured request, can send it again, and a wallet with no age limit pays it again. NIP-47 only defines the `expiration` tag, so this is a safety rule no spec requires, like SAFE-03. It passes when the wallet ignores or rejects the request, and it's skipped when the relays drop the old event themselves.

**SAFE-07** runs only when the wallet advertises `create_connection`. It asks for a connection with 1 sat more budget than the current connection has left, and for one with a method the current connection lacks; both must be refused. Each request uses a client key that nwc-check throws away, so even a connection the wallet wrongly creates can't be used. If one is created, the result gives its wallet pubkey so you can delete it.

## Invoices

| ID | Severity | Level | Check |
|---|---|---|---|
| CORE-01 | HIGH | MUST | make_invoice matches amount, description and expiry |
| CORE-02 | MEDIUM | MUST | make_invoice sets the description_hash |
| CORE-03 | MEDIUM | MUST | lookup_invoice finds by payment_hash and invoice |
| CORE-04 | HIGH | MUST | lookup_invoice shows pending, then settled |
| CORE-05 | LOW | MUST | Unknown invoice returns NOT_FOUND |

These checks decode the returned BOLT11 invoice rather than trusting the reply's fields. CORE-01 asks for a 5000-second expiry, not a round default like an hour, so a wallet that ignores `expiry` and uses its own default still fails. They create unpaid invoices in the wallet; only CORE-04 needs a counterparty, to pay one.

## Payments

| ID | Severity | Level | Check |
|---|---|---|---|
| PAY-01 | HIGH | MUST | make_invoice returns a payable invoice |
| PAY-02 | HIGH | MUST | pay_invoice delivers the payment |
| PAY-03 | HIGH | SAFETY | pay_invoice returns the matching preimage |
| CORE-07 | LOW | SHOULD | pay_invoice reports plausible fees_paid |
| CORE-08 | CRITICAL | MUST | pay_invoice pays the amount given for an amountless invoice |

## Connection budget (NWC-13)

| ID | Severity | Level | Check |
|---|---|---|---|
| EXT13-01 | LOW | MUST | get_budget is listed in the info event and get_info |
| EXT13-02 | MEDIUM | MUST | get_budget is empty or has every required field |
| EXT13-03 | LOW | MUST | A renewing budget has renews_at, a never budget doesn't |

These read [NWC-13](https://github.com/nostr-wallet-connect/nwc/blob/main/13.md) `get_budget` and are skipped when the wallet doesn't advertise it.

## Ledger

| ID | Severity | Level | Check |
|---|---|---|---|
| CORE-09 | HIGH | SAFETY | Balance drops by the amount plus fees_paid |
| EXT13-04 | HIGH | SAFETY | used_budget rises by the amount plus fees_paid |

Both share one payment and run on their own after the other sections. CORE-09 compares `get_balance` before and after it, and EXT13-04 compares `used_budget` from `get_budget`. Each allows up to 1 sat of difference for wallets that round to whole sats.

## Budget

| ID | Severity | Level | Check |
|---|---|---|---|
| SAFE-02 | CRITICAL | SAFETY | Concurrent payments stay within the budget |
| SAFE-08 | CRITICAL | SAFETY | Every spending method respects the budget |

These run last because they use up the connection's budget, so use a connection with a small budget.

**SAFE-02** reads the connection's remaining budget with `get_budget`, or from `--budget-sats` for wallets without it. It then sends five payments at once, sized so four fit the budget and five don't, and checks how much the counterparty actually received. The five payments together must fit under `--max-sats`.

**SAFE-08** runs after SAFE-02. With each advertised spending method (`pay_invoice`, `pay_keysend`, `multi_pay_invoice` and `multi_pay_keysend`), it tries to pay the counterparty 1 sat more than the budget has left. Each must be refused with `QUOTA_EXCEEDED`; the check fails if the counterparty receives anything. Other errors, such as `INSUFFICIENT_BALANCE`, don't show whether the budget was checked, so they make the check skip rather than pass. Keysend payments need the LND counterparty to run with `--accept-keysend`.

## Lightning address counterparty

A Lightning address can be paid but can't pay, and its LUD-21 `verify` URL says whether an invoice was paid, not how much arrived. So with `--counterparty-address`:

- PAY-01 and CORE-04 are skipped: they need the counterparty to pay the wallet.
- CORE-08 is skipped: it needs an invoice without an amount, which LNURL-pay never gives.
- SAFE-04 is skipped: it would pass on an overpayment it can't see.
- SAFE-08 only tries `pay_invoice` and `multi_pay_invoice`, since a keysend to the service can't be looked up.
- If the service has no `verify` URL, PAY-02 and the other checks that look for a payment are skipped. PAY-03 still checks the wallet's preimage against the invoice.
