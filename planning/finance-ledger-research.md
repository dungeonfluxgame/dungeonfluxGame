# Finance ledger, funding and accounting: design note (F round 0f)

Date: 2026-10-25. Author: launch agent F (money core and accounting owner). Status: design only; no
code changed. Revised after the round-0 critic (REVISE, 26 items), the round-0b critic (16 items,
round 0c), the round-0c critic (5 points, round 0d), the round-0d critic (4 changes, round 0e), the
round-0e critic (3 changes, round 0f) and the
coordinator rulings (a)-(g)
of 2026-10-25. The mapping of each critic item to its section is in
`artifacts/tmp/agents/F/round-0b.md`, `round-0c.md`, `round-0d.md`, `round-0e.md` and `round-0f.md`. Each implementation round after this one is critic-reviewed
before the coordinator commits it.

Governing text: `planning/master-launch-plan.md` (cited "plan N.N"), the coordinator decisions in
`artifacts/tmp/agents/LAUNCH.md` (cited "D1".."D32"), the ownership change of 2026-10-24 that moved
the money core from B to F (D27), D29-D31 (lock order, F's admission endpoint, funding cursor and
fresh `member_snapshot`), D28 (P's wasm dashboards), `planning/coding-style.md`, `AGENTS.md`,
`planning/commerce-service.md`, `planning/service-operations.md` (recovery journal), and the peer
notes `planning/account-service-research.md` (B; its funds sections 8-9 are superseded by this
note), `planning/hosted-runtime-research.md` (O1; its 10.2 "Needs from F" is binding and section
11.4 answers it item by item), `planning/asset-storage-research.md` (O2; 11.5 and 12.6 answer it),
`planning/campaigns-and-table-research.md` (S1; 17.2 and 18.7), the cost engine
`crates/df-tools/src/gameplay/cost.rs` (C) and the tariff survey
`artifacts/tmp/agents/H6/tariff-inventory.md` (cited "H6 n").

Values marked **(proposed)** are owner-pending. Every one of them ships as configuration with the
stated default (plan 19: "keep these as explicit configuration decisions").

## 0. Coordinator rulings applied in this revision (2026-10-25, final)

| Ruling | Effect in this note |
|---|---|
| (a) Promotional credit is presented net by default, configurable | `promotional.presentation = net`; gross mode stays a switch (5, 16.5, 23) |
| (b) One trial per email and per account | as designed (7.4) |
| (c) Accounting time zone UTC by default, configurable; cut-off is the posting date | 17.8 |
| (d) Customer-caused content-screen rejections stay company loss at launch; a per-account upload rate limit is the abuse control | 11.1, 15.1 (the limit is O2's `request_upload` rate limit; F counts screens per account) |
| (e) Pending provider routes are refused in hosted paid mode until C meters them; Jev is metered under D21, so C closes that route first | 15.2 |
| (f) Statements may show quantity plus charge per line, under D26 | 18.9 |
| (g) Promotional balances are forfeited at closure with a posting; refunds for lots past the processor window are a manual operator payout through a dedicated entry kind | 14.4, 16.18, 16.19 |

## 1. Scope, plan sections and acceptance ids

| Plan section | What it fixes here |
|---|---|
| 2.2, 2.4 | Price formula: table time x platform rate + billable provider units x pinned tariff x markup; versioned schedule; uncharged classes (failed validations, duplicates, defects, unused speculation) are company expense |
| 3.1 | Vocabulary: account credit, session allocation, pot (not an owner), reservation, quote |
| 4.1 | Meter states; rational accumulator; no rounding per second; remainders carried |
| 4.2, 4.3 | Four customer controls (hourly ceiling, session ceiling, individual authorization, per-request threshold), in-flight holds count, pacing; presets Story/Immersive/Cinematic/Custom (D4) |
| 5.2 | Purchase flow: credit only after a confirmed processor state; purchase risk limits |
| 5.3-5.5 | Allocations, consent version, epochs, proportional split, largest remainder, withdrawal, departure, party disputes |
| 6.1 | Five distinct balances |
| 6.2 | Admission sequence and the attempt record |
| 6.3 | Settlement table; 15-minute customer hold; late results company-funded |
| 6.4 | Integer micro-dollars; whole-cent refunds; explicit rounding entries |
| 6.5 | Append-only double-entry journal; purchased versus promotional credit |
| 6.6 | State-based processor reconciliation; out-of-order events never resurrect credit |
| 6.7 | Refunds to original method; failed refund goes to review; tax evidence per lot |
| 7.1-7.3 | Default protections; customer statements; daily reconciliation; manual adjustments |
| 9.7-9.11, 9.13 | Billing dashboard amounts, add-credit, session funding, in-game chip, session summary, operator console data |
| 13.8-13.9 | Storage quotas are storage limits (D10), not money; storage is company cost |
| 13.7, 13.10 | Financial state is forward-only; restore needs reconciliation before paid work reopens |
| 15.1, 15.2, 15.4 | Entities, operations (all mutations idempotent; timeouts return an operation reference), invariants |
| 17 | PAY-01..06, POT-01..06, JOB-01..05, TIME-01..03, REF-01..03, OPS-02, OPS-04, and the money halves of SAVE-02, SAVE-03, UX-02, UX-04, AUTH-02, OPS-01 |

Section 17 has no `LEDGER-*` or `REFUND-*` ids: the refund ids are `REF-01..03`, and the ledger
invariants come from plan 15.4 and 6.5. Section 26 names a test for every one of them.

## 2. Ownership after D27

| Area | Owner | Notes |
|---|---|---|
| Identity, sessions, portal pages, router, payment-processor adapter (Stripe Checkout behind a port, faithful fake), webhook inbox, purchase orders and payment attempts, operator pages | B (migrations 0005-0007) | B verifies and re-reads processor objects, then hands authoritative state to F |
| Journal, chart of accounts, five balances, lots, trial grant, pots, allocations, consent, quotes, reservations, attempts' money states, settlement, time pricing, credits, refunds and disputes accounting, tax fields, price versions, company-loss classes, reconciliation, accounting exports | F (migrations 0020-0024) | This note |
| Customer and operator dashboards (wasm) | P (D28) | Calls F's view APIs through `df-portal/src/app_api/` |

Superseded in B's note: 2 decisions 17 (tranche prepaid draw) and 19-23 (B's trial tables), 4.5
`PrepaidDraw`, 5.2 `wallet`, `prepaid_reservation`, `run_totals`, `ledger_entry`, `usage_cursor`,
`usage_debit`, `platform_loss`, `payer_grant`, `spend_pace`, 5.3 `refund_request`, and sections
8.4-8.7 and 9. D2 replaced the tranche with a durable reservation per operation; this note is that
design. B keeps `checkout`/purchase order, payment attempt and `gateway_inbox` in 0006 and the
operator tables in 0007.

## 3. Crate layout

### 3.1 Choice: extend `df-commerce` (pure), add `df-finance` (native facade), store in `df-persistence`

| Crate | Kind | Owns | Depends on |
|---|---|---|---|
| `df-commerce` (existing) | pure, no I/O, no clock, wasm-safe | chart of accounts, journal entry type, its balance rule and the entry-kind allowlist (a mirror of the SQL seed, 17.6), one posting constructor per flow, the return rule per lot state, lots and draw order, trial policy, pots and the exact split, price versions, tariff lookup and units coverage, quote arithmetic, the four ceilings and pacing, admission decision, settlement decision, meter pricing, refund and dispute plans, rounding, company limits, the finance plan (entries plus row changes), customer-safe and operator view types in separate modules | `df-types`, `serde` |
| `df-finance` (new) | native, async | `FinanceService` facade (the API of section 18), consumer-owned ports, owned workers (settlement, deadlines, outcome consumers, funding feed consumer, meter holds and pricing, reconciliation, integrity job, period close, exports), tunables loader, dev view, test fakes | `df-types`, `df-commerce`, `df-observe`, `tokio`, `serde_json` |
| `df-persistence` (existing) | native store | `finance_store`: the SQL definer functions (17.6), the cross-schema admission transaction with O1's `fence_current` and O2's `admit_generation`, migrations 0020-0024, PostgreSQL contract tests | adds `df-finance` (implements its port), like it implements `df-customer`'s |

Why extend `df-commerce` rather than add a pure `df-ledger` crate:

1. Plan 15.3 names `df-commerce` for "pure money, consent, quote, and exposure policies" and asks
   that allocations be "a carefully modeled extension to existing commerce scopes". A second pure
   money crate would be a second place to look for money rules (coding style 1.2).
2. `df-commerce` already has the evidence types the journal needs (`ObservationAuthority`,
   `RefundOutcome`, `OperationId`) and only depends on `df-types`, so the wasm dashboard (D28) can
   use its view types directly. A separate crate would add an edge to every consumer for nothing.
3. The journal *model* (accounts, entries, the balance rule, posting constructors) is pure value
   logic; only storing it is persistence, which plan 15.3 gives to `df-persistence`.

Why a native `df-finance` facade rather than putting the service in `df-customer`: accounts and
money are separate capabilities (commerce-service "Independent authority"; auth D01-D03) and now
separate owners (D27); `df-customer` stays B's. The pattern is B's: pure policy crate, native
facade with consumer-owned async ports, store adapter in `df-persistence`, composition wires them.

The division of labour at the database boundary (17.6): Rust decides (pure plans over snapshots read
under locks); PostgreSQL enforces (every write goes through a definer function that checks the plan
against an allowlist, so a Rust defect cannot mint credit and a database role cannot post at all).

### 3.2 What happens to today's `df-commerce` modules

- `subscription.rs`, `allowance.rs`: untouched (subscriptions are deferred, plan 1.2).
- `spend.rs` (six supplier-exposure counters): kept as the company supplier-exposure policy, with
  one amendment so the owner's rule holds: `SpendCounter.limit: Money` becomes
  `limit: ExposureLimit` (`Unlimited | AtMost(Money)`), and the doc sentence "Both platform period
  caps are mandatory" goes. Hosted defaults: platform day, platform month and supplier account are
  `Unlimited` unless configured (company provider caps are configuration, default off, LAUNCH hard
  rules); tenant/payer and campaign are `Unlimited` on the supplier side (customer protection is
  the retail side, section 10); job is `AtMost(the quote's supplier maximum)`. The change lands in
  the same round as S4's acceptance crate, which constructs `SpendCounter` (PROTOCOL "keep the
  shared tree compiling").
- `credit.rs` (`CreditState`, one wallet per tenant) and `ports.rs` (`CommerceRepository`,
  `PaymentGateway`): not used by hosted finance. Lots plus the journal replace the wallet (section
  7), because a single wallet cannot carry per-lot refund sources, per-lot tax, disputes that
  freeze one lot, or purchased versus promotional classes. `PaymentGateway` is B's adapter port;
  `CommerceRepository` is superseded by `df-finance`'s `FinanceStore`. Both retire once S4's
  tests pass on the new service (D30, settled). Their rules survive as posting preconditions:
  credit only from verified-latest processor state, a refund keeps its hold until a conclusive
  outcome, Unknown never releases.

### 3.3 New `df-commerce` modules (round 1, pure)

| Module | Public items (main) |
|---|---|
| `ledger` | `Book { General, Promo, Exposure }`, `AccountCode` (closed chart, section 5), `AccountKey`, `JournalEntry`, `EntryKind`, `Line`, `EntryRefusal` |
| `entry_rules` | `EntryKindRule` table: for each kind its allowed (code, side) pairs, stream, anchor, external reference kind, actor kinds and reversibility; the SQL seed is generated from it and a test compares both (17.6) |
| `postings` | one `fn` per flow in section 16, each returning `Result<JournalEntry, EntryRefusal>` |
| `returns` | `ReturnTarget { Allocated, Available, Frozen, ChargebackRecovery, Refuse }`, `return_target(lot_state, allocation_status)` (14.4): the one rule every release, surplus, credit and refund-return uses |
| `funding` | `LotId`, `LotClass { Purchased, Promotional(PromoSource) }`, `LotState`, `draw_order`, `TrialPolicy`, `TrialDecision` |
| `pot` | `AllocationId`, `Epoch`, `SplitMode { Proportional, Sponsored, Equal }`, `split_exactly` (largest remainder, stable tie-break), `split_capped` (water-fill at exhausted payers) |
| `rounding` | the rounding rules of section 4 as functions: `round_up_micro`, `round_down_micro`, `round_half_up_micro`, `cumulative_cents` |
| `price_schedule` | `PriceVersion`, `PriceVersionId`, `PriceVersionLabel` (opaque), `PlatformRate`, `Markup` (rational), `Tariff` (with context tier), `Presets`, `retail_maximum`, `retail_charge`, `supplier_cost` |
| `tariff` | `PriceTable::parse` and `parse_bill` moved from C (29.3); `units_cover_route`, `category_priced` |
| `quote` | `QuoteInput`, `Quote` (estimate and maximum, retail and supplier), `QuoteRefusal` |
| `ceilings` | `CustomerCeilings`, `CeilingWindow`, `PaceBucket`, `WorkClass { Essential, Optional, Premium }`, `check_ceilings` |
| `admission` | `AdmissionSnapshot`, `AdmissionRequest`, `AdmissionPlan`, `AdmissionRefusal`, `decide_admission` |
| `settlement` | `SupplierOutcome`, `Usability`, `HoldState`, `SettlementPlan`, `decide_settlement` |
| `meter` | `MeterAccumulator`, `PricedInterval`, `price_interval` (with the hold cap, 13.2) |
| `refund` | `RefundPlan`, `plan_refund`, `RefundRefusal` |
| `dispute` | `DisputePlan`, `plan_dispute_open`, `plan_dispute_close` |
| `plan` | `FinancePlan { event_kind, entries, changes }`, `RowChange` (closed enum of typed after-images, 17.6), the only thing the store applies |
| `company_limits` | `ExposureLimit`, `CompanyLimits` (defaults all `Unlimited`) |
| `views::customer`, `views::operator` | two modules with no shared types (section 18.9) |

## 4. Money representation and rounding

Rules below are cited by number: "4.7" means rule 7 of the list in this section.

- Amounts are integer micro-dollars (`df_types::Money`, micro-units, one currency tag). Columns are
  `bigint` named `*_micro_dollars` with `CHECK (... >= 0)` except signed journal lines. `bigint`
  bounds one amount at about $9.2 trillion; every conversion `u128 <-> i64` at the store boundary
  is checked and a failure is a typed `MoneyError::Overflow`, never a truncation. No float appears
  anywhere (plan 6.4); the usage report type has no float field and no dollar field that F uses
  (15.3).
- USD only at launch; every lot, session, price version and account key carries a currency, and
  every entry balances per book and per currency, so a second currency cannot mix silently.
- Rational rates: the markup is a ratio (`3/2`, D4), tariffs are `usd/per` from the price file
  converted to an exact nano-dollar ratio, the platform rate is micro-dollars per hour.
- **F prices from units.** The runtime and dispatcher report exact provider units per category
  (tokens, characters, seconds, credits, calls). F multiplies units by the pinned tariff row
  rationally. C's per-line `micro_usd` (half-up per line, H6 7) is evidence only: F stores it,
  compares it, and counts a disagreement of more than one micro-dollar per line in
  `df_finance_cost_book_disagreement_micro_dollars_total`; it never becomes a charge or a cost.
- Rounding rules (each one documented on the pricing page and in the statement help):
  1. A retail **maximum** (a hold) rounds **up** to a whole micro-dollar.
  2. A retail **charge** for one usage event is the exact rational sum of its lines (units x pinned
     tariff x markup) rounded **down** to a whole micro-dollar once, then capped at the
     reservation's maximum.
  3. A **supplier cost** for one operation is the exact rational sum of its lines (units x pinned
     tariff, no markup) rounded **half up** once. Rounding per line (C's way) is not used.
  4. Time is charged by the cumulative rule of section 13: per session,
     `charged(t) = floor(priced_ms(t) * rate / 3_600_000)`, so each priced interval charges the
     difference and remainders carry forward; no second or minute is rounded up (plan 4.1).
  5. A payer's share of a charge is a largest-remainder split in micro-dollars: `floor` each, then
     one micro-dollar at a time to the largest fractional remainders, ties to the lowest id.
     Shares sum exactly to the charge and never raise it (plan 5.4).
  6. **The supplier cost split between 5100 and 5110** uses the same rule:
     `split_exactly(cost, [purchased part of the charge, promotional part of the charge])`, ties to
     purchased. A charge of zero (no usable output) has no split: the whole cost is 5200.
  7. Cash refunds are whole cents, rounded **down** per lot. The sub-cent purchased remainder of a
     closing account (under 10,000 micro-dollars) is released by one `rounding` entry (16.9); it
     does not block closure (14.4). Repeated micro-refunds cannot gain anything.
  8. Screens show cents: balances available to spend round down; charges and totals round half up
     from the exact amounts; the view carries both the exact micro-dollars and the display cents
     (section 18.9).
  9. **Reconciliation tolerances** (21): processor and bank amounts are whole cents and compare
     exactly (tolerance zero). Supplier invoice lines compare with
     `supplier.invoice_line_tolerance_micro_dollars = 10_000` per line and
     `supplier.invoice_total_tolerance_bps = 50` per invoice, whichever is larger; a difference
     inside the tolerance is posted on the invoice match to the one rounding account (5700) and
     recorded as `within_tolerance`, not as a difference; outside it, it is a difference.
  10. **Exports** (23.4): detail files carry exact micro-dollars. Summary files carry cents by the
      cumulative rule: for each account, the day's cents are `R(cumulative since period start) -
      R(cumulative through the previous day)` with `R` = round half up to the cent, so the daily
      figures sum to the monthly figure. One line to the single rounding account (5700) per day
      balances each day's journal in cents.

## 5. Chart of accounts

Three books. **General** holds real money and obligations: it is the ledger (trial balance,
statements). **Promo** holds promotional credit, which has no cash value. **Exposure** holds supplier
commitments that are not yet liabilities. Promo and Exposure are memoranda, not part of the general
ledger, and every export marks them so (23.1). Every entry balances per (book, currency) and the
balance check is a database constraint, so a line cannot cross from one book to another: the
entry `Dr promotional_offset / Cr supplier_exposure_open` does not balance in either book and cannot
commit. Signed lines: positive is a debit, negative a credit. "Natural balance" is the balance times
the account's normal side; accounts marked **enforced** keep a balance row whose natural balance can
never go below zero (section 17.1).

| Code | Name | Book | Kind | Normal | Dimensions | Enforced |
|---|---|---|---|---|---|---|
| 1000 | `bank_operating` | General | asset | debit | none | no |
| 1100 | `processor_clearing` | General | asset | debit | processor | no |
| 1200 | `supplier_prepaid` | General | asset | debit | provider | no |
| 1300 | `dispute_funds_withheld` | General | asset | debit | processor | no |
| 2110 | `purchased_available` | General | liability | credit | payer, lot | yes |
| 2120 | `purchased_allocated` | General | liability | credit | allocation, lot | yes |
| 2130 | `purchased_reserved` | General | liability | credit | allocation, lot | yes |
| 2140 | `purchased_frozen` | General | liability | credit | lot | yes |
| 2300 | `supplier_payable` | General | liability | credit | provider | no |
| 2310 | `supplier_accrued_unconfirmed` | General | liability | credit | provider | no |
| 2400 | `tax_payable` | General | liability | credit | jurisdiction | no |
| 2500 | `refund_payable` | General | liability | credit | lot | yes |
| 3000 | `owner_equity` | General | equity | credit | none | no |
| 4100 | `revenue_table_time` | General | revenue | credit | none | no |
| 4110 | `revenue_ai_usage` | General | revenue | credit | none | no |
| 4190 | `refunds_of_consumed_service` | General | contra-revenue | debit | none | no |
| 4200 | `promotional_revenue` | General | revenue | credit | promo source | no (gross mode only) |
| 5100 | `cost_of_services` | General | expense | debit | provider | no |
| 5110 | `cost_of_services_promotional` | General | expense | debit | provider | no |
| 5200 | `absorbed_supplier_cost` | General | expense | debit | provider, loss class | no |
| 5300 | `promotional_credit_expense` | General | expense | debit | promo source | no (gross mode only) |
| 5400 | `processing_fees` | General | expense | debit | processor | no |
| 5500 | `chargeback_loss` | General | expense | debit | processor | no |
| 5700 | `rounding` | General | expense | debit | none | no (either sign); the one rounding account |
| 2210 | `promotional_available` | Promo | memo liability | credit | payer, lot | yes |
| 2220 | `promotional_allocated` | Promo | memo liability | credit | allocation, lot | yes |
| 2230 | `promotional_reserved` | Promo | memo liability | credit | allocation, lot | yes |
| 3910 | `promotional_offset` | Promo | memo | debit | promo source | no |
| 9100 | `supplier_exposure_open` | Exposure | memo | debit | provider | **no** (reconciled instead) |
| 9190 | `supplier_exposure_offset` | Exposure | memo | credit | provider | no |

- Promo sources (closed): `trial`, `goodwill`. Loss classes (closed): section 15.
- **9100 is not enforced and keeps no balance row.** Every admission and settlement on the platform
  touched one `account_balance` row per provider before, which serialized the platform. Open
  exposure is instead the sum of `provider_attempt.supplier_exposure_micro_dollars` over attempts
  whose supplier state is `ExposureOpen` (an indexed partial sum), and the daily subledger
  reconciliation proves the sum of the 9100/9190 lines equals it (21). A configured company limit
  uses `company_exposure_counter` (11.2), not an account balance.
- Customer service-credit liability (balance 2) is 2110 + 2120 + 2130 + 2140; 2500 is cash owed
  back. Promotional credit (2210-2230 against 3910, Promo book) is reported beside it, never as
  deferred revenue (plan 6.5: no cash refund value, cannot become purchased credit).
- Unconstrained shared accounts (revenue, expense, clearing, offsets, exposure) keep **no** balance
  row, so settlements do not contend on one hot row; their balances are sums over indexed lines,
  read by reconciliation and exports.
- Per-reservation and per-attempt detail lives in subledger rows (`reservation_share`,
  `provider_attempt`), tied to the account keys by commit-time triggers (17.3) and reconciled daily
  (21).

## 6. The five balances (plan 6.1)

| Balance | Computed from | Shown to |
|---|---|---|
| 1. Cash collected | 1000 + 1100 + 1300 (General) | operator |
| 2. Customer service-credit liability | 2110 + 2120 + 2130 + 2140 (purchased, General); promotional 2210-2230 (Promo book) reported separately | operator; each payer sees their own parts |
| 3. Session allocation | 2120 + 2220 keyed by allocation (a reclassification inside 2, not extra money) | payer (own), table (pot totals) |
| 4. Retail reservation | 2130 + 2230 keyed by allocation, with per-reservation rows in `reservation_share` | payer (own holds), table (held total) |
| 5. Supplier liability | 2300 + 2310 (General) + open exposure including Unknown (Exposure book, from the attempt rows) | operator only |

A company-sponsored failed request keeps its supplier exposure after its customer reservation is
released, exactly the case plan 6.1 calls out. The operator console also shows the treasury figure
plan 6.1 asks for: purchased liability + refund payable + supplier payable and accrual + open
exposure + funds withheld by disputes, against cash collected (a view, not a cap).

## 7. Funding lots, purchases and the trial grant

### 7.1 Lots

A **funding lot** is one source of credit for one payer account: a confirmed purchase (class
`Purchased`) or a promotional grant (class `Promotional(trial | goodwill)`). Row `finance.funding_lot`
(section 17.2) records class, payer, currency, original amount, the processor references
(processor, payment reference, charge reference) for purchased lots, the tax treatment and billing
location evidence captured at purchase (plan 6.7), state and expiry. Balances are never columns:
they are the lot's account keys (2110/2140/2500 for purchased, 2210 for promotional, plus the
allocation keys that carry the lot dimension). The lot row also carries the running totals that
make the conservation equation checkable at commit (17.3): `consumed`, `refunded`, `charged_back`,
`rounded` and, for promotional lots, `expired` and `forfeited` micro-dollars.

Lot states: `Active`, `Frozen` (dispute open), `ChargedBack` (a dispute was lost for the lot's
whole unrefunded amount, so nothing backs any liability still on the lot; 14.2), `Closed` (every
key zero, nothing held or payable), and for promotional lots `Forfeited` (account closure). A
**partial** dispute loss leaves the lot `Active` with its `charged_back_micro_dollars` total raised
(14.2). Spendability is a function of balances and state together: only the `*_available` keys of
an `Active` lot can be drawn into an allocation (7.2). What returns to a lot in each state is the
one table of 14.4; refunded and disputed lots are never spendable (plan 7.1).

### 7.2 Draw order (decision 3, plan 5.3)

When credit moves out of available into an allocation, and when a reservation takes from an
allocation, lot portions are taken in this fixed order: promotional lots first (oldest first; the
one that would expire first if expiry is ever enabled), then purchased lots oldest first. Oldest
first leaves the newest purchases unspent, which is what a refund of "original unspent purchase
lots" returns (refunds take newest first, section 14.1). Credit returning to an allocation or to
available goes back to the lot it came from, so the order never has to be reversed. Only lots in
state `Active` are drawn from.

### 7.3 Purchases (B hands F the authoritative state)

B's webhook worker re-reads the processor object (plan 5.2 step 5, 6.6) and calls
`apply_processor_state(PaymentState)` (18.6), never a redirect or an event body. F compares the
state with what it has recorded for that payment reference and posts only the difference:

- first sight of a captured payment: create the lot and post `purchase` (16.2);
- refunded total above what F has recorded: our own refunds are matched by refund reference; the
  rest is an `external_refund` (16.14);
- dispute status changes: section 14.2;
- fee and payout facts arrive through B's balance-transaction reads (21.2).

**Serialization.** The call is one transaction: `enter_service()`, the `payer` row, then the
`processor_payment_state` row for (processor, payment reference), both `FOR UPDATE` through the
`lock_flow` definer (19.1). The incoming state is merged into the recorded one with monotone
rules: captured only turns true, the refunded total is the maximum, the dispute status only
advances. If the merge changes nothing the call returns `AlreadyCurrent` and writes nothing. It
never raises on a stale or repeated state, so an out-of-order webhook is not a poison message, and
two concurrent applies for one payment queue on the row lock and the second sees the first's
result. The update is the conditional `UPDATE ... WHERE object_revision = $read`, which can only
fail through a bug; the monotonic trigger of 17.2 is the backstop, not the mechanism.

The refunded total, dispute status and captured flag are therefore monotonic in F's record: an
older state read after a newer one changes nothing (PAY-03). A payment seen refunded before it was
ever seen captured posts the purchase and the refund in one transaction, so spendable credit is
never created (PAY-03). The business key is (processor, payment reference): a second webhook event
for the same payment under a new event id finds the posting and changes nothing (PAY-02). A pending
payment is never a lot (UX-02). The `purchase` posting needs the `processor_payment_state` row as
its anchor; without one the database refuses the entry (17.6).

Purchase risk limits (plan 5.2, proposed): $100 per purchase, $200 outstanding purchased credit,
$200 per day per new payer (new = first purchase within 30 days). B calls
`purchase_limits(account, amount)` before creating a checkout; F answers from the ledger. These
are fraud and risk controls on buying credit, not company provider caps.

### 7.4 The trial grant (decision 3, ruling (b))

- $10.00 promotional credit (`trial.amount_micro_dollars = 10_000_000`), granted on email
  verification, one per account and one per verified email address (ruling (b); stricter than D3's
  "every new verified account", confirmed), spent first, no expiry, no cash value, not refundable,
  never moved into purchased credit, not transferable (plan 3.1).
- B's verification transaction calls `grant_trial(tx, account, email_grant_key)` as its **first**
  statement (finance rows first, D25). `email_grant_key` is an HMAC-SHA256 of the normalized
  address under a durable secret held by B (S2 secrets layout); B keeps that key past account
  deletion, so a deleted and re-registered address gets nothing. It holds no address in clear.
- Unique `trial_grant(account_id)` and unique `trial_grant(email_grant_key)`. A refused grant is
  `TrialDecision::AlreadyGrantedToAccount | AlreadyGrantedToEmail` with one audit line; it never
  fails verification. The `trial_grant` posting needs the `trial_grant` row as its anchor (17.6).
- The lot is `Promotional(trial)`; posting 16.1. The billing dashboard shows it as "Trial credit"
  until spent (plan 9.7 "promotional credit"). The operator dashboard and exports report trial
  consumption at retail and the supplier cost of trial-funded usage (5110) separately.
- Presentation is net by default (ruling (a)): promotional consumption is not revenue; only the
  supplier cost of promotional usage is expense. Gross mode is the switch of 16.5.

## 8. Pots, allocations, consent and epochs (plan 5.3-5.5)

### 8.1 Session funding

`finance.session_funding` is created when the table session is first funded or started
(`open_session_funding`, 18.8) and pins the **price version** for the session's life (plan 2.2: a
provider price change does not silently reprice an active session). It carries the session policy
(revisioned): the preset or custom hourly ceiling, session ceiling, per-request threshold, split
mode, whether contributor names and amounts are visible to the table (plan 5.3 group setting,
default off), the generation categories the host permits, and the pacing bucket state. A new
session of the same campaign gets a new row and the then-current version (J04: "fresh session
price/consent"; an old session's authorization never funds a new one). Its status is `open`,
`settling` (13.4) or `closed`.

### 8.2 Allocations and consent

- An **allocation** is one payer's revocable authorization for one session: amount authorized (the
  individual authorization of plan 4.2), status (`active`, `paused_departed`,
  `consent_incompatible`, `withdrawn`, `closing`, `closed`), and its consent.
- **Consent** (`finance.allocation_consent`, append-only, revisioned) records the price version, the
  maximum hourly ceiling and session ceiling the payer approved, the permitted generation settings
  digest, the split mode, the payer's own per-job approval threshold, the consent-terms text version
  shown, and the time (plan 5.3 "Record the consent version and a timestamp").
- Allocating posts 16.3: available -> allocated per lot in draw order. It is not revenue and moves
  no ownership (plan 5.3). A payer may raise their own allocation (new consent revision); nobody
  else can (plan 7.1 "never increase a payer's individual maximum because the host increased the
  session maximum").
- A payer must be an active member or contributor of the campaign (S1 `member_snapshot` with
  `authorized`, 3.2 role matrix: contributors without a seat may allocate) and verified (B), and
  their payer standing must be active (section 19.4). **The first allocation a payer makes in a
  campaign also creates F's feed position row for that campaign, in the same transaction, from the
  `event_seq` and `feed_epoch` of the `member_snapshot` that allocation read** (S1 0h; 17.2), so a
  `left` that commits after the snapshot is applied once by the consumer and the catch-up sweep can
  never overwrite the position.
- **Withdraw** (J03, POT-02): releases only the unreserved part of the allocation back to available
  (16.13); holds stay attached to their reservations. The allocation becomes `withdrawn`.
- **Departure** (J07, POT-04): F's durable `funding` consumer of S1's `campaign_event` feed (17.2,
  18.7) sets `PausedDeparted` on `left`/`removed`, starts a new epoch without that payer and
  releases the unreserved part. In-flight reservations keep their shares and settle normally. A
  leaver can explicitly sponsor again (new consent). Host and ownership transfers change nothing in
  finance (D11, plan 5.5 "a new host does not inherit authority over cards or balances").
- **Policy raised above consent** (POT-03): a policy revision above an allocation's consented
  ceiling marks it `ConsentIncompatible` and starts a new epoch without it; it rejoins when the
  payer consents to the new policy. Lowering a ceiling needs no consent and immediately constrains
  new admissions; it never cancels sent work (plan 4.2).
- **No stranded allocated credit.** Credit that comes back to a payer through an allocation whose
  status is not `active` or `consent_incompatible` (that is `paused_departed`, `withdrawn`,
  `closing`, `closed`) lands in `*_available`, never in `*_allocated` (the `return_target` rule,
  14.4). As a second guard, every release and settlement that resolves the last open reservation
  of such an allocation ends with `sweep_allocation`: if `reserved = 0` and `allocated > 0`, the
  same transaction moves allocated to available (`allocation_release`). An allocation may become
  `closed` only with `allocated = 0` and `reserved = 0` (a deferred trigger, 17.3), and the
  reconciliation reports any non-zero key of a closed allocation (21.6). `end_session_funding`
  (13.4) moves every allocation to `closing`, and the settlement-finish step closes them after the
  sweep, so a session's last hold cannot leave credit behind.

### 8.3 Epochs and the exact split

- An **epoch** fixes the set of participating allocations and their weights. Proportional mode
  (default): weight = authorized amount at epoch start (plan 5.3: $20/$10/$10 -> 50/25/25).
  Sponsored: one allocation, weight 1. Equal: weight 1 each. Sponsored and equal need every
  participating payer's consent to that mode.
- A new epoch begins on: an allocation added, raised or withdrawn, a departure, a consent
  incompatibility or its cure, a split-mode change. It affects only later reservations (POT-05);
  every reservation stores its own shares (`reservation_share`), and a settlement charges against
  exactly those shares (plan 5.3 "never retrospectively redistribute").
- `split_exactly(amount, weights)` (`df_commerce::pot::split_exactly`, the one production
  function; every test of the split calls it, 26.1): `floor(amount * w_i / W)` each, then the
  remainder one micro-dollar at a time to the largest fractional remainders, ties to the lowest
  allocation id. Deterministic; sums exactly; no share exceeds its exact value by a full
  micro-dollar.
- **An exhausted payer** (remaining authorization smaller than their share): `split_capped` caps
  that share at the remaining authorization and splits the uncovered part among the others by the
  same rule, repeatedly (water-fill). Nobody ever pays more than their own authorization. This is
  configuration `split.on_exhausted_payer = water_fill | refuse` (default `water_fill`) because it
  departs from a strict proportional share at exhaustion (question 28.9).

## 9. Price versions and quotes

### 9.1 Price versions

- Source: owner-editable `content/price-schedule.json` (schema `dungeonflux.price-schedule/1`),
  validated at load with a per-knob fallback and an audit line (coding style 8.9). Publishing a
  version is an operator command that inserts an immutable `finance.price_version` row and its
  tariff rows (and creates the version's provider-dimension account keys, 17.6); rows are never
  edited (a trigger refuses it), only retired.
- A version holds: id, an **opaque label** (`PriceVersionLabel`, a short code derived from the id,
  never a rate, a date pattern with a rate or a human phrase containing one, 18.9), effective-from,
  currency, platform rate (default $2.00 per active table-hour, D4), markup (default 3/2, D4), the
  tariff snapshot (one rate per provider, model, category and context tier, copied from
  `content/pricing.json` at publish time with its digest and dates), tax policy version, presets
  (Story $3/h, Immersive $8/h, Cinematic $20/h, Custom bounds, D4), the supported product
  categories, and the flag `proposed_pilot = true` until the owner approves prices (D4: shown as
  "proposed pilot pricing"; the views carry the flag).
- `content/pricing.json` stays C's supplier-cost table (what calls actually cost). The version's
  tariff is the customer price basis (plan 2.2). They start equal; they may diverge.
- Round 1 checks the first version's tariff against H6's inventory (H6 3c, 4, 8): every category a
  priced route declares needs a row (9.2 and the test `every_priced_route_category_has_a_tariff_row`).
- **Context tiers (H6 U7).** `price_version_tariff` carries `context_tier_max_tokens` (null =
  unbounded). Until C's table stores tiers, each row is stored with the tier it is quoted for (272,000
  tokens for the `gpt-6-luna` and `gpt-5.5` rows) and F reserves at the highest-rate row that
  applies to the request's maximum units; a request whose maximum input exceeds every stored tier
  is refused `CategoryUnpriced` (long context is unpriced, not guessed).

### 9.2 Retail and supplier amounts

- **A missing price refuses; it never prices at zero.** Admission checks, for the pinned version, that
  every category in the request's `maximum_units` has a tariff row for the route's provider and
  model (`CategoryUnpriced { category }`), and that `maximum_units` covers every category the
  route declares (`MaximumUnitsIncomplete { category }`), including cached-input, cache-write,
  image-input and call-count categories (H6 3c). Customer-billed work is refused. Company-paid
  work on a route with no row follows 15.2 (a configured per-route supplier maximum, never charged
  to a customer).
- Supplier maximum of an operation: the route's conservative maximum units per category priced with
  the pinned tariff at the highest applicable tier, rounded up.
- Retail maximum: the same maximum units priced with the pinned tariff, times the markup, rounded
  up; plus at-consumption tax when that mode is on.
- Retail charge: actual units x pinned tariff x markup, rounded down once, capped at the retail
  maximum. Supplier cost: actual units x pinned tariff, rounded half up once (section 4).
- Priced units come only from the pinned table. A usage report with no units (dollars only), or one
  whose price came from a hard-coded or float path (H6 5, 6: the picture, SAM and live Kling paths),
  is rejected as `UsageWithoutUnits`, or accepted only as `Estimated` units priced from the pinned
  row (15.3).
- Table time is priced only by section 13.

### 9.3 Quotes (estimate versus maximum authorized, plan 4.2, 9.2, 9.10, J06)

- `quote(QuoteRequest)` returns an **estimate** (expected units) and a **maximum** (the ceiling the
  reservation will hold), both retail, plus the payer's own estimated share, the price version
  label, the expiry (`quote.lifetime_seconds = 600`, proposed) and the cancellation policy code.
  The supplier maximum and the breakdown are stored, never returned to customers (D26).
- A quote is stored (`finance.quote`) with the canonical request digest and the version. The same
  request digest within the lifetime returns the same quote (idempotent).
- **A quote is single-use.** `reservation.quote_id` is unique where not null, and the admission
  transaction moves the quote `approved -> used` with a conditional update on the locked quote row
  (`WHERE state = 'approved' AND expires_at_ms > now`) in the same transaction that creates the
  reservation (rank 4 of the lock order, 19.1). A second admission with the same quote finds it
  `used` and is refused `QuoteAlreadyUsed`; an admission that is refused or rolled back leaves the
  quote `approved`. A quote whose maximum is below the admission's computed maximum, or whose
  digest differs, is refused `QuoteMismatch`. One approved $5 quote therefore funds one operation.
  Unused approved quotes expire at `expires_at_ms` (deadline worker).
- **Per-request threshold**: an operation whose retail maximum is above the session's threshold
  (default $0.50, proposed; customer-set) is admitted only with an approved quote. Approval:
  the requesting host or co-host, plus every payer whose estimated share is above that payer's own
  consented approval threshold; a payer who declines is excluded from that one operation
  (`quote_approval` rows). Premium cinematics are always quoted (plan 2.2). A quote that expires
  before approval is refreshed at the then-current version, never silently admitted (J06).
- Routine work below the threshold is admitted under the session's generation-policy consent
  without a stored quote; the admission computes the same maximum.

## 10. Customer ceilings, presets and pacing (plan 4.2, 4.3)

These are customer controls and are built as specified; none of them is a company provider cap.

| Control | Rule at admission (all must hold) |
|---|---|
| Table hourly ceiling | `window_spend + new_maximum <= hourly_ceiling`, where `window_spend` = every open hold of the session regardless of age + charged amount of every reservation admitted in the last 60 minutes and already settled + time holds; a settled reservation counts once at its admission time, never as both hold and charge |
| Session ceiling | all charges + all open holds of the session (setup work included) + new maximum <= session ceiling |
| Individual authorization | each share <= that allocation's remaining authorization; enforced again by the nonnegative `purchased_allocated`/`promotional_allocated` balances |
| Per-request threshold | maximum above threshold -> approved quote required (9.3) |

- Presets set the hourly ceiling (Story $3, Immersive $8, Cinematic $20, Custom customer-chosen
  within `presets.custom_minimum`/`custom_maximum`, proposed $1-$100 per hour). The ceiling includes
  the platform fee, so Story leaves $1/h of AI (plan 4.3); the presets' category behaviour (voice,
  pictures, video off) is the generation policy owned by B/S1/O1 surfaces, read by the runtime.
- **Pacing** (plan 4.2 "avoid spending the entire hour's budget in the opening minute"): a token
  bucket per session in retail micro-dollars, capacity = hourly ceiling, refill = ceiling per hour,
  starting at `pacing.initial_burst_permille = 250` of capacity. `Optional` work (pictures, music,
  prefetch) needs bucket tokens >= its maximum and must leave the **essential reserve** free: the
  next meter interval's time charge plus `pacing.essential_reserve_micro_dollars = 100_000`
  (proposed $0.10). `Essential` work (required narration, the rules-bearing text call) bypasses the
  bucket but never the hard ceilings. `Premium` (quoted) work bypasses the bucket once approved.
- **Funds state** of a session, read by O1's `TableGate`: `Funded`; `Low` when the funded runway
  at the selected maximum is under `funds.low_runway_minutes = 10` (O1 maps it to EssentialOnly);
  `FundingRequired` when the next meter interval or the essential reserve cannot be held.
- **Runway** (plan 4.3) is a range: "at selected maximum" = available pot / hourly ceiling;
  "at your recent rate" = available pot / trailing 30-minute charge rate. Never shown as a promise.

## 11. Admission and the attempt record (plan 6.2, D2)

### 11.1 Who is admitted

Every supplier call passes this boundary, including company-paid work (plan 4.2 "no backend worker
can bypass these limits"). The payer and the principal are separate:

- `Payer::Table(session)` for customer-billed work; `Payer::Company(class)` for company-paid work
  (content screens, D13; moderation and quality checks; memory maintenance; speculative prefetch
  inside the company's own speculation policy). Company admissions hold supplier exposure only
  and check company limits (default unlimited).
- `AdmissionPrincipal::Campaign { tenant, campaign, fence }` for everything the runtime asks: the
  fence is mandatory and the receipt principal is the campaign. `AdmissionPrincipal::Account
  { account }` for the fence-free company path used by the control plane for **content screens of
  uploads** (account scoped, no campaign, no owner): the caller is O2's upload pipeline running as
  `df_control`, not a runtime. The path skips `fence_current` and media admission (steps 7 and 8 of
  11.2) and keeps everything else: `enter_service()`, the reconnect gate, `ServiceHalting`, company
  limits, the journal-age gate, the receipt (principal = account) and the attempt row. Only
  `Payer::Company(content_screen)` may use it; any other payer with an account principal is refused
  `PrincipalNotAllowed`.
- **The account-principal lifecycle (content screens), end to end.** An attempt with an account
  principal has `account_id` set and `campaign_id` null (17.3). Its steps, each a journaled event
  (17.6):
  1. *Admit*: as above; the reservation is a zero-retail company reservation with the exposure
     lines; the entry's ledger stream is an **`account` stream** (`stream_kind = 'account'`, the
     uploader's account id), not `platform` and not `payer`: it is locked only by that account's
     own screens, and O2's upload rate limit (20 per hour per account) bounds its rate, so it never
     becomes a shared chain head.
  2. *Prepare*: `prepare_dispatch_account(attempt)` and `cancel_prepared_account(attempt)`, granted to
     `df_control` (the caller is O2's pipeline in the control plane, which has no placement), check
     only that the attempt is an account attempt of the stated kind and state. They replace the
     `leased_campaign()` check of the campaign variants.
  3. *Send, unsent, outcome*: `mark_attempt_sent`, `mark_attempt_unsent` and
     `record_transport_outcome` take a **null fence for an account attempt**: `fence_current` and
     `dispatcher_serves_campaign` are skipped (there is no campaign owner), `current_dispatcher()`
     is still required and recorded, and the dispatcher put into the attempt is the only caller
     allowed to report its outcome. A null fence on a campaign attempt is refused.
  4. *Usage*: the dispatcher's `record_transport_outcome` carries the provider-returned units
     (12.1); there is no runtime `record_usage` and no usability (a screen is company-paid and
     always settled as `content_screen`, 12.2), so the facts are complete at the transport
     outcome.
  5. *Deadlines*: `reserved_expiry`, `prepared_expiry`, `sent_reconcile`, `customer_hold` (not
     applicable: no customer hold), `usage_missing` and `unknown_resolution` apply by time only;
     the fence-supersession clause of `reserved_expiry` does not.
  6. *Settle*: `company_loss` with class `content_screen`, posted to the account stream.
  The test `a_content_screen_runs_admit_prepare_send_outcome_settle_and_each_expiry_without_a_campaign`
  drives it all (26.1 JOB-10).
- Customer-caused screen rejections stay company loss at launch (ruling (d)): a screen call is
  always settled as `content_screen` company loss whatever it finds. The abuse control is the
  per-account upload rate limit in O2's `request_upload` (default 20 per hour); F counts screens
  per account in `df_finance_content_screens_total{result}` and exposes the figure to the
  operator console so the owner can see a free-generation abuse path if one appears.

### 11.2 The admission transaction

One PostgreSQL transaction at `READ COMMITTED` (19.1), about 10 ms (D2), committed before anything
is sent. It runs in F's finance endpoint on the control plane's internal mTLS listener (D30) under
role `df_finance`; every statement below that locks or writes is a definer call (17.6), because the
role holds no table privilege except `SELECT`.

1. `df_runtime.enter_service()` (first statement, before any row lock), `SET LOCAL lock_timeout`,
   `SET LOCAL transaction_timeout`. A halt in progress raises `DF002`, mapped to the retryable
   `ServiceHalting`.
2. **Idempotency, read inside `lock_flow` after the locks of step 3.** (The receipt must not be read
   before the locks: two identical concurrent requests would both miss it.) `finance.operation_receipt`
   (`namespace = admission`, principal = campaign or account, operation id = the runtime's effect
   operation id) holds a fingerprint of the canonical request **including `attempt_id`** (attempt,
   effect, payer, route, work class, maximum and estimate units, quote, media admission, request
   digest). A receipt exists **only for an admitted operation**: refusals are never stored, so a
   retry after a top-up, a fence change or a lock timeout is evaluated afresh and cannot replay a
   stale refusal. A repeat of an admitted operation returns the stored outcome (SAVE-02); the same
   operation id with a different fingerprint is `Conflict`. Where the request has a session row
   (a table payer) the session lock serializes two identical requests, and the second finds the
   first's receipt. Where it has none (a company or account principal) nothing serializes them
   before the write, so the **unique key is the serialization point**: the receipt insert and the
   `attempt_id` unique key conflict, `commit_event` raises `23505`, the store maps it to "already
   admitted", rolls the transaction back and re-reads the receipt in a new one, returning the
   stored outcome (or `Conflict` if the fingerprint differs). A unique violation is never returned
   to the caller as an error.
3. **Locks in finance order** through `finance.lock_flow('admission', ...)` (19.1): the
   `session_funding` row `FOR UPDATE` (table payer), the quote row `FOR UPDATE` when a quote is
   given, the epoch's allocation rows in allocation-id order, their lot rows `FOR SHARE` in lot-id
   order (admission reads lot state and writes no lot row; settlement and the flows that change a
   lot total take `FOR UPDATE`, 19.1). The definer then reads the receipt (step 2) and returns the
   rows it read under those locks as the snapshot.
4. **Checks** (plan 6.2), no further locks: payer standing mirror active (19.4); **a fresh
   `member_snapshot(account, campaign)` per payer** (S1 5.1; one lock-free statement, so no
   campaign lock is taken before `fence_current` and D29 holds). A payer whose snapshot is not
   `authorized` (not a member or contributor, or the account is closing or closed) is **excluded
   from the shares**; the resulting shares are recorded on the reservation, and if no payer is
   left the refusal is `FundingRequired`. There is no feed-lag gate: check-at-use replaces it (D24,
   D31); lot states spendable; allocations active; consent revisions compatible; price version
   equals the session's; quote valid, unexpired, approved, matching and unused (9.3); route
   metered (a `Metering::Pending` route is `RouteUnmetered`, 15.2); every category priced and
   covered (9.2); the four ceilings and pacing (section 10); the recovery journal's unconfirmed age
   under its bound (22.3). Company limits: with every scope `Unlimited` (the default) nothing is
   read or locked; a configured scope locks its `company_exposure_counter` row after the lots and
   runs `spend::propose_spend` with `ExposureLimit` (3.2). Configuring a company cap therefore
   serializes admissions on that counter; that cost is stated where the knob is documented.
5. **Pure decision**: `df_commerce::admission::decide_admission(snapshot, request)` returns the
   `FinancePlan`: retail maximum and shares (split 8.3, each share drawn from the allocation's lot
   portions promotional first), supplier maximum, the journal entry (16.4), the row changes
   (reservation, shares, attempt in `Reserved`, quote `used`, receipt, work items).
6. **Post and write, before the fence and media steps.** `finance.commit_event(plan)` (17.6) takes the
   ledger-stream and balance rows in sorted order (19.1 ranks 8 and 9), applies the row changes
   and posts the entry. Posting first means the balance-row locks are taken before any O1 or media
   lock, so the order finance -> O1 -> media (D25, D29) holds for every flow, including O2's
   `cancel_unclaimed_effect` and F's 24 h resolution, which also post before they touch media.
   An insufficient pot fails here, on the nonnegative balance check, as `FundingRequired`.
7. **Owner fence** (campaign principal only): `df_runtime.fence_current(tenant, campaign, epoch,
   claim_seq, placement, 'paid')` (O1 6.1). False -> `FenceStale` or `PaidWorkFrozen`, and the
   transaction rolls back, which undoes step 6 and releases its locks.
8. **Media** (media kinds only): O2's `media_catalog::admit_generation(&tx, ...)` after the
   finance and O1 rows (D25). `NotAdmitted(InFlight | Ready | AudienceEmpty | QuotaFull)` rolls
   back the whole transaction and is returned as the refusal (O2 6.2). Nothing of ours needs to
   survive this rollback, because refusals write no receipt (step 2).
9. Commit. The event is already in the recovery journal's pending table (17.6).

Refusals (`AdmissionRefusal`, closed; each one holds nothing and writes nothing but an audit line):
`FundingRequired`, `CeilingReached(Hourly | Session | Individual)`, `PaceDeferred` (optional work
only; the caller uses its prepared fallback), `QuoteRequired`, `QuoteExpired`, `QuoteNotApproved`,
`QuoteAlreadyUsed`, `QuoteMismatch`, `ConsentRequired`, `PriceVersionMismatch`, `RouteUnmetered`,
`CategoryUnpriced(category)`, `MaximumUnitsIncomplete(category)`, `FenceStale`, `PaidWorkFrozen`,
`ServiceHalting`, `RecoveryJournalBehind`, `CompanyLimit(scope)` (only when configured),
`PrincipalNotAllowed`, `MediaNotAdmitted(reason)`, `StoreBusy` (lock timeout), `Conflict`. The
transient ones (`FundingRequired`, `PaceDeferred`, `FenceStale`, `PaidWorkFrozen`, `ServiceHalting`,
`RecoveryJournalBehind`, `StoreBusy`, `CeilingReached`) are marked `retryable` in the type so the
caller knows a later retry is meaningful. A lost commit acknowledgement is `Unknown { operation }`
with the lookup path (plan 15.2); the caller retries with the same operation id and either finds
the receipt or is evaluated afresh.

### 11.3 Attempt states (three dimensions)

O2's third critic found that `CustomerReleased` cannot be terminal: the supplier side lives on.
So `finance.provider_attempt` has three state columns, and the attempt is terminal only when all
three are:

| Dimension | States |
|---|---|
| `dispatch_state` | `Reserved` -> `DispatchPrepared` -> `Sent` -> `ResultKnown`, `OutcomeUnknown` or `VerifiedUnsent`; `Reserved` or `DispatchPrepared` -> `CancelledUnsent`; `OutcomeUnknown` -> `ResultKnown` (a late result); `Sent` or `OutcomeUnknown` -> `FailedUnknown` (after the configured limit, measured from `sent_at`, D25) |
| `customer_state` | `Held` -> `Charged` or `Released`; `Charged` -> `Credited` (partly or fully) |
| `supplier_state` | `ExposureOpen` -> `CostRecorded` or `ExposureReleased` (confirmed not billed or never sent) or `AbsorbedWorstCase` (written off, 12.4) |

- `Reserved -> DispatchPrepared`: `finance.prepare_dispatch(attempt, fence)` when the runtime claims
  the effect (plan 6.2 "persist DispatchPrepared before outbound network activity"). It checks the
  caller with `df_runtime.leased_campaign()` against the attempt's campaign and records the
  current fence.
- `DispatchPrepared -> Sent`: the egress dispatcher calls `finance.mark_attempt_sent(attempt,
  fence)` (11.4); then it journals `Dispatching` through O2's writer and sends (O1 10.3). Only
  admission and send take the paid fence.
- `Sent -> VerifiedUnsent`: `finance.mark_attempt_unsent(attempt)` (11.4): the dispatcher proved no
  request bytes were handed to the HTTP client. The hold and the exposure are released in the same
  call. Terminal, and a state in which a retry with a new attempt is safe.
- `Reserved -> CancelledUnsent`: `cancel_unclaimed_effect(tx, attempt)`, conditional on `Reserved`
  (a user cancel or O2's sweeper). If the state is `DispatchPrepared` or later at the time of the
  call the answer is `AlreadyClaimed` and nothing changes.
- `DispatchPrepared -> CancelledUnsent`: only the deadline worker past
  `dispatch.prepared_expiry_seconds` (12.3), or the claiming runtime itself through
  `cancel_prepared(attempt)` when it abandons the effect before asking the dispatcher. By O1 10.3 a
  `Sent` row is committed before any byte leaves, so `DispatchPrepared` definitely means not sent;
  both paths lock the attempt row, so a concurrent `mark_attempt_sent` either wins (the cancel then
  finds `Sent` and does nothing) or loses (the dispatcher is refused `AttemptNotPrepared` and
  sends nothing).
- Transport outcomes from the dispatcher (`record_transport_outcome`): `ResultKnown`,
  `OutcomeUnknown` (timeout or lost response; JOB-01), or a typed failure before any send
  (`FenceStale`, `PaidWorkFrozen`, `ServiceHalted`, `CircuitOpen`), which the dispatcher reports
  through `mark_attempt_unsent` when the row is already `Sent`, or the runtime through a cancel
  while it is still `DispatchPrepared`.
- A second admission or dispatch of one attempt is refused by `UNIQUE (attempt_id)` and the state
  checks (JOB-01, JOB-02). Retrying an Unknown attempt needs reconciliation or a provider
  idempotency key, never a new attempt id (J09).

### 11.4 The dispatcher contract: O1 10.2 item by item

F is authoritative for the attempt table, its states, grants and policies (O1 10.2). Every
function a dispatcher calls verifies the **dispatcher**, with `df_runtime.current_dispatcher()`
(O1 10.2 item 6); a dispatcher holds no placement token, so `leased_campaign()` is not used for
them. Functions the runtime calls verify `leased_campaign()` as before. Each F transaction that
uses the fence or a directory function begins with `enter_service()`, is bound by the reconnect
gate (22.2) and the 5 s `idle_in_transaction_session_timeout` on every F role (O1 6.1), and maps
`DF002` to the retryable `ServiceHalting` refusal.

| O1 10.2 | F's answer |
|---|---|
| 1 `dispatcher_instance_id` | column on `provider_attempt` (bytea 16), written by `mark_attempt_sent` from `current_dispatcher()`, never from a parameter |
| 2 `mark_attempt_sent` | locks the attempt (rank 7), requires `current_dispatcher()` not NULL, requires `df_runtime.dispatcher_serves_campaign(tenant, campaign)` (O1 definer: the campaign's current placement `worker_id` equals the calling dispatcher's `worker_id`; request to O1), requires `DispatchPrepared`, calls `fence_current(..., 'paid')`, writes `Sent`, `sent_at_ms` (database clock) and the instance. It takes no finance row above the attempt |
| 3 `VerifiedUnsent` | new `dispatch_state`; `mark_attempt_unsent(attempt)` allowed only for `Sent` with `dispatcher_instance_id = current_dispatcher()`; it follows the full lock order (session, allocations, lots, then the attempt, 19.1) because it releases the reservation (16.6) and the exposure in the same call |
| 4 Sent reconciliation | work item `sent_reconcile` (12.3): an attempt `Sent` for more than `dispatch.sent_reconcile_after_seconds` (300) or whose instance is no longer live (`df_runtime.dispatcher_is_live(instance)`, an O1 definer granted to F) becomes `OutcomeUnknown` (`dispatch_outputs` cannot exist for it: O1 writes that row in the same transaction as `ResultKnown`, so F never has to look). **Ruling (2026-10-25): F does not ask a dispatcher to look an attempt up.** D23 gives the control plane no route to a dispatcher and F holds no provider credential, so an unknown attempt is settled by the supplier reconciliation of 12.4 and 21.3 (invoice or usage match), or written off at 45 days. `record_lookup_outcome` of round 0b is withdrawn, and `dispatch_output_exists` is no longer requested of O1 |
| 5 24 h limit | applies to `Sent` as to `OutcomeUnknown`, measured from `sent_at_ms`; the result is `FailedUnknown`, customer credited per D21 |
| 6 `record_transport_outcome` | checks `current_dispatcher() = attempt.dispatcher_instance_id`; carries the provider-returned usage units (12.1); runs **in the caller's transaction** (no autonomous commit), because the dispatcher writes `df_runtime.dispatch_outputs` in the same one, after the call (finance row, then O1 row) |
| 7 reconstruction | `reconstruct_campaign` endpoint ending in `df_runtime.record_money_state` (22.2); there is no `clear_paid_work_freeze` |
| 8 pricing | F prices `[opened_at, closed_at)` of each closed segment minus credits; `proven_until`, `unproven_ms` and the old `meter_forgone` tail rows are dropped (13.2); `forgone_revenue` remains for `meter_unfunded` and `refunded_externally` |

Grants (17.9): `df_campaign_runtime` -> `meter_funded_until`, `session_funding_state`,
`prepare_dispatch`, `cancel_prepared`, `record_usage`; `df_egress` -> `mark_attempt_sent`, `mark_attempt_unsent`,
`record_transport_outcome` (all with an account-attempt variant, 11.1); `df_control` -> `attempt_status`,
`cancel_unclaimed_effect`, `attempt_outcomes_after`, `start_readiness`, `open_session_funding`,
`end_session_funding`, `personal_usage_statement`, `prepare_dispatch_account`, `cancel_prepared_account`; `record_money_state` stays O1's, granted to
`df_finance`.

**Which dispatch definers are journaled.** All of them. `prepare_dispatch`, `cancel_prepared`,
`mark_attempt_sent`, `mark_attempt_unsent`, `record_transport_outcome` and `record_usage` are
`commit_event` kinds (`attempt_prepared`, `attempt_prepare_cancelled`, `attempt_sent`,
`attempt_unsent`, `transport_outcome`, `usage_recorded`); each carries the attempt row's
after-image (a few hundred bytes) and is inserted into the journal's pending table in the same
transaction (17.6). None of them is awaited: the dispatcher still waits only for its own
`Dispatching` entry before it sends (O1 10.3), so F adds no latency to the send path. What a
restore can lose is only the unconfirmed tail; after a replay an attempt whose later events are
missing is conservatively re-derived as `OutcomeUnknown` when the journal holds its
`Dispatching` entry (22.2 step 2), and otherwise stays in the last state the journal confirms
(`Reserved` or `DispatchPrepared`, both definitely unsent, which the expiry items cancel).

### 11.5 What O2's sweeper reads: `attempt_status`

`finance.attempt_status(attempt) -> AttemptStatus` is a definer read (no lock, one statement)
granted to `df_control`. It maps F's three dimensions to O2's `ProviderAttemptState`
(`catalog_ports.rs`), so O2 does not learn F's states:

| F state | O2 `ProviderAttemptState` |
|---|---|
| `Reserved` | `Reserved` |
| `DispatchPrepared` | `DispatchPrepared` |
| `Sent` | `Sent` |
| `OutcomeUnknown`, customer `Held` | `OutcomeUnknown` |
| `Sent` or `OutcomeUnknown` with customer `Released` (supplier outcome unknown) | `CustomerReleased` |
| `ResultKnown` | `ResultKnown { succeeded, at_ms }` |
| `FailedUnknown` | `FailedAfterReconciliation` |
| `CancelledUnsent`, `VerifiedUnsent` | `Cancelled` (nothing was sent; O2 rejects with `not_dispatched`) |

`CustomerReleased` never implies the supplier outcome is known (O2's question): it is returned only
while the dispatch state is `Sent` or `OutcomeUnknown`. `cancel_unclaimed_effect(tx, attempt)`
returns the typed `CancelOutcome::{Cancelled, AlreadyClaimed}`: `Cancelled` also for an attempt that
is already `CancelledUnsent` or `VerifiedUnsent` (idempotent), `AlreadyClaimed` for `DispatchPrepared`
or later. It runs in the caller's transaction (F rows first, then O2's quota and asset rows).

## 12. Settlement (plan 6.3, D21, D22)

### 12.1 Facts

Settlement is keyed by the shared attempt id and never needs a live campaign owner (coordinator
contract for O1 0d). Three facts arrive separately and are recorded on the attempt:

- **Supplier outcome** (`SupplierOutcome`): `Billed { units, source: Reported | Estimated }`; `NotBilled`
  (provider refused before billable work, confirmed); `Unknown`. **The dispatcher supplies the
  provider-returned usage units** in `record_transport_outcome` (it is the one that reads the
  provider response; token counts, seconds and credits come back with it), so the runtime need
  supply only usability and any units the provider does not return (for example the characters it
  sent, which are `Estimated`, 15.3). Units from the dispatcher win over the runtime's. If neither
  source reports units, the `usage_missing` deadline applies (12.3).
- **Usability** (`Usability`):
  - *Media* (pictures, voice files, music, video): O2's `FinalizeOutcome { attempt, Ready |
    Superseded | Rejected(reason) | Invalid(reason) }` from `media.settlement_outbox` (D22; only
    O2's terminal transition writes it, 12.6). **There is no usability deadline for media.** A
    video may sit in O2's quarantine and review for up to 20 h; booking it as
    `usability_unreported` company loss at 300 s would give away work that later arrives. Media
    waits for O2's `FinalizeOutcome`, which O2 guarantees through its own deadlines (staging
    deadline, `quarantine_max_age`, F's 24 h outcome event); F raises an alert, never a loss, if
    none has arrived after `media.finalization_alert_hours = 48` (proposed).
  - *Non-media* (text and speech): the runtime's report with the usage:
    `Usable | NoUsableOutput(class)`. A missing report after
    `usability_report_deadline_seconds = 300` (proposed) **counted from `result_at`** counts as no
    usable output (customer not charged, company loss `usability_unreported`, alert).
- **Hold state and its clock.** `Held` until settled, or `Released` by a deadline (12.3). The
  hold clock starts at **`sent_at_ms`**, the database time at which `mark_attempt_sent` committed
  (not at admission, not at dispatch preparation). A result that is known but awaits O2's
  finalization (`ResultKnown`, billed, media) has its own rule: the hold stays `Held` for
  `hold.finalization_wait_minutes = 60` (proposed) counted from `result_at_ms`, then `Released`
  with the notice "Technical delay - not charged"; a `Ready` that arrives after that release is a
  late result, company funded (JOB-05).

### 12.2 The settlement table (`decide_settlement`, pure)

| Situation | Customer | Supplier | Plan / decision |
|---|---|---|---|
| Billed, usable, delivered | charge actual retail (<= maximum), release surplus (16.5) | cost to 5100 (purchased share) and 5110 (promotional share) | 6.3 row 1 |
| User cancelled before dispatch | release (16.6) | release exposure | 6.3 row 2 |
| User cancelled after dispatch | `accepted_job_policy` (default `charge_if_delivered`, proposed): charged only if a usable result is delivered, never above maximum | keep exposure until known | 6.3 row 3 |
| Provider refused before billable work (confirmed `NotBilled`) | release | release exposure | 6.3 row 4 |
| Billed, no usable output (provider failure, invalid, output lost after provider success, superseded, duplicate, platform defect) | release (never charged) | cost to 5200 with the loss class (16.7) | 6.3 row 5, D21, D22, O2 critic hole 2 |
| Unknown | hold shown as unresolved until the deadline, then released (12.3) | worst case stays in the exposure book until reconciled (12.4) | 6.3 row 6, JOB-01 |
| Speculative asset never used | not charged (it was company-admitted) | cost to 5200 `speculative_unused` | 6.3 row 7 |
| User-requested alternate | a new quote and operation | separate cost | 6.3 row 8 |
| Actual supplier cost above the supplier maximum | customer charge capped at retail maximum | excess cost to 5200 `overrun_beyond_quote`, incident alert | commerce-service "explicit platform loss/overrun" |
| Content screen of an upload (company-paid, any finding) | none | cost to 5200 `content_screen` | D13, ruling (d) |

Charging a share: within each payer share, the promotional portion is consumed before the purchased
portion (decision 3); across shares, the charge is split by largest remainder over the held share
amounts. The supplier cost is split between 5100 and 5110 by `split_exactly` over the charge's
purchased and promotional parts (section 4 rule 6). Every release and surplus lands where the
return rule of 14.4 says, so a Frozen or ChargedBack lot never receives spendable credit.

### 12.3 Deadlines and the work queue (owned worker, injected clock)

All time-driven transitions are **work items**: a row in `finance.work_item (work_id, kind, ref_id,
due_at_ms, claimed_by, lease_until_ms, generation)` written in the transaction that creates the
condition (for example, `mark_attempt_sent` writes the `sent_reconcile`, `customer_hold` and
`unknown_resolution` items). A worker claims due items with `FOR UPDATE SKIP LOCKED`, a lease and a
generation, so a delayed completion from an old claim cannot overwrite a newer one (plan 6.6), then
**resolves the attempt to its session with an unlocked read, takes the locks of 19.1 in order, and
re-checks the state**: a stale item is a no-op. The worker never claims by locking the attempt
row. The queue row is rank 0 of the lock order (nothing waits behind it, 19.1).

| Item | Condition | Action |
|---|---|---|
| `reserved_expiry` | attempt `Reserved` for more than `reserved.expiry_seconds = 300`, or its recorded fence is no longer the campaign's current owner for more than `reserved.superseded_expiry_seconds = 60` (a lock-free read, `df_runtime.owner_fence(tenant, campaign)`, request to O1) | cancel as `CancelledUnsent`: release hold (16.6) and exposure, write the outcome event for media (12.6). Covers every attempt kind, company admissions included, so a runtime that crashes after admission cannot leave a hold, exposure or (for media) a quota reservation behind |
| `prepared_expiry` | attempt `DispatchPrepared` for more than `dispatch.prepared_expiry_seconds = 120` | the same cancel. `Sent` is committed before any byte leaves (O1 10.3), so `DispatchPrepared` is definitely unsent; a dispatcher arriving later is refused `AttemptNotPrepared` |
| `sent_reconcile` | attempt `Sent` past `dispatch.sent_reconcile_after_seconds = 300`, or its instance not live | `OutcomeUnknown` with the worst-case exposure kept. **There is no dispatcher lookup** (D23 has no route from the control plane to a dispatcher): an unknown attempt is settled through the supplier reconciliation of 12.4 and 21.3, or written off (11.4 item 4) |
| `customer_hold` | attempt `Sent` or `OutcomeUnknown` with no settlement, `hold.unresolved_release_minutes = 15` (proposed; per route override `hold.release_minutes.<route>`, proposed 45 for video) after `sent_at_ms` | customer `Released` (16.6); the views show "Technical delay - not charged". Supplier exposure stays |
| `finalization_wait` | `ResultKnown`, billed, media, no `FinalizeOutcome`, `hold.finalization_wait_minutes = 60` after `result_at_ms` | customer `Released`, as above |
| `usability_deadline` | non-media, supplier outcome `Billed`, no usability report 300 s after `result_at_ms` | `NoUsableOutput(usability_unreported)` and settle |
| `usage_missing` | attempt `ResultKnown` with no usage units from either source (the dispatcher's `record_transport_outcome` or the runtime's `record_usage`), `usage.report_deadline_seconds = 300` after `result_at_ms`, for any attempt kind | treat the supplier outcome as `Unknown`: customer `Released` (the hold would otherwise strand, and `session_finish` would never complete), exposure kept at the worst case, resolution by supplier reconciliation (12.4). The dispatch state stays `ResultKnown`; no new arrow |
| `unknown_resolution` | attempt `Sent` or `OutcomeUnknown` more than `unknown.resolution_hours = 24` (proposed, D25) after `sent_at_ms` | `FailedUnknown`; the customer is not charged (the hold was released at 15 minutes); exposure stays at the worst case (12.4); **F writes the attempt outcome event (12.6) in the same transaction** and O2's consumer rejects the asset. F calls nothing in O2 |
| `exposure_write_off` | exposure unmatched `supplier.exposure_write_off_days = 45` after `sent_at_ms` | accrue at the worst case (12.4) |
| `quote_expiry` | approved or open quote past `expires_at_ms` | `expired` |
| `session_finish` | `session_funding.status = 'settling'` | the finish check of 13.4, repeated every `meter.settling_poll_seconds = 30` |
| `settle` | all facts of an attempt complete, or a deadline above fired | one settlement transaction (12.2) |

- **Late result** (JOB-05): a result after customer release is never charged to the customer; its
  supplier cost goes to 5200 `late_result_after_release`; the output may still be delivered
  (company-funded).

### 12.4 Supplier exposure after an unknown outcome

Exposure stays at the worst case (the attempt's `supplier_exposure_micro_dollars`) until the
supplier-usage reconciliation (21.3) matches the attempt against the provider's usage or invoice:
billed -> cost to 5200 `unknown_outcome`; not billed -> exposure released. Unmatched after
`supplier.exposure_write_off_days = 45` (proposed, after one invoice cycle) it is accrued at the
worst case (5200 against 2310) with a reversal path if evidence appears later. The customer is
never charged for it. The write-off books the **worst case**; the owner is asked to confirm that
(28.7).

### 12.5 Credits after a charge (JOB-04, plan 6.3 row 5)

`credit(CreditRequest)` (18.4): `ReverseCharge { usage_event, amount, reason }` returns up to the
remaining reversible amount of that charge to its original lots (16.8), once per operation. The race
that two concurrent credits with different operation ids both pass a sum check is closed three ways:

1. `usage_event.reversed_micro_dollars` with `CHECK (reversed_micro_dollars <= retail_micro_dollars)`;
2. the credit flow takes the `usage_event` row `FOR UPDATE` (rank 7, 19.1) before it reads the
   remaining reversible amount, so credits for one charge are serialized, and a second credit sees
   the first's total;
3. `reservation_share.reversed_micro_dollars` with `CHECK (reversed <= charged)` per payer share.

**Split across payer shares.** The credit amount is split with `split_exactly` over the shares'
remaining reversible amounts (`charged - reversed`), so no payer is credited more than they were
charged and the parts sum to the credit. Each part returns to that share's lot by the 14.4 rule
(Active: available; Frozen: frozen; Closed: the lot reopens, Closed -> Active under the payer
lock; ChargedBack: the whole credit is refused `CreditTargetChargedBack` and the operator may
grant promotional goodwill instead).

**Reclassification 5110 -> 5200** (reason `TechnicalFailure` only): the supplier cost already
booked to 5100 and 5110 (stored on the usage event as `supplier_purchased_micro_dollars` and
`supplier_promotional_micro_dollars`) moves to `absorbed_supplier_cost{provider, platform_defect}`
in proportion to the reversal. The cumulative target after a credit is
`floor(supplier_cost x reversed_total / retail_charge)`; the entry moves the difference between that
target and `usage_event.reclassified_micro_dollars`, split between 5100 and 5110 by `split_exactly`
over their remaining unreclassified amounts. A full reversal therefore reclassifies the whole
cost exactly. `Goodwill { payer, amount, case }` is a promotional grant (`goodwill` source) by an
operator with a reason (plan 7.3).

### 12.6 Contract with O2 (assets, catalog, sweeper)

**What F reads of O2.** F consumes `media.settlement_outbox` through O2's definer
`media.settlement_outcomes_after(after_seq, since_at_ms, limit)` (request to O2: `EXECUTE` to
`df_finance`). The outbox sequence is a `bigserial`, so a lower number can commit after a higher
one; F does not rely on gaplessness. The consumer is an **idempotent rescan**: it reads
`outbox_seq > position`, and on every pass also the rows whose `at_ms` is within
`outbox.rescan_window_minutes = 15` of the newest it has seen (greater than any transaction
timeout, validated at load); each row is applied by `UNIQUE (attempt_id)` on
`finance.settlement_fact`, so a re-read changes nothing. The position
(`settlement_feed_position`) is a hint, not a guarantee. A daily reconciliation (21.4) lists every
O2 terminal attempt older than the window with no `settlement_fact` and applies it. Request to
O2: stamp `at_ms` from the database clock inside the terminal transaction.

**O2's reason names are used verbatim.** F maps them, with `Invalid(...)` and `Rejected(...)` both
carrying O2's label:

| `FinalizeOutcome` | Usability in F | Loss class if billed |
|---|---|---|
| `Ready` | `Usable` | none (charged) |
| `Superseded` | `NoUsableOutput` | `superseded` |
| `Invalid(over_maximum_bytes)`, `Invalid(validation_failed)` | `NoUsableOutput` | `invalid_output` |
| `Rejected(output_lost)`, `Rejected(staged_lost)`, `Rejected(staged_expired)`, `Rejected(supplier_failed)`, `Rejected(screen_unavailable)` | `NoUsableOutput` | `failed_no_output` (output lost after provider success is never charged) |
| `Rejected(not_dispatched)`, `Rejected(unknown_outcome_failed)` | F-originated (12.3); a received row is checked against F's own state | none |
| `Rejected(screen_rejected)`, `Rejected(review_expired)`, `Rejected(abandoned)`, `Rejected(quota_exceeded)` | upload paths: no paid generation; ignored for money | none |

The exact label is kept on the attempt (`usability_class`) so the audit trail and the operator
view show O2's reason, and a new O2 reason that F does not know is an alert and is treated as
`failed_no_output` (the customer is never charged for an unrecognized failure).

**What O2 reads of F.** `attempt_status` (11.5). The 24 h failure and every F-side unsent
cancellation (deadline cancel, `VerifiedUnsent`) are **one path: F's attempt outcome event**, which
O2 already designed a consumer for. `finance.attempt_outcome_event (event_seq, attempt_id UNIQUE,
media_asset_id, outcome, reason, recorded_at_ms)` is written in the same transaction as the
transition (`outcome` in `failed_unknown`, `cancelled_unsent`; `reason` in `unknown_outcome_failed`,
`not_dispatched`). O2's consumer reads it through `finance.attempt_outcomes_after(after_seq,
since_ms, limit)` (granted to `df_control`; the same idempotent rescan rule as above, applied once
per attempt) and in its own transaction moves the asset to `Rejected(<reason>)`, releasing the
quota reservation (F rows first, then quota, then asset, D25). F does **not** call
`media_catalog::reject_attempt`; that ask of round 0 is withdrawn.

**Personal usage statements.** O2's export worker takes F's section through
`finance.personal_usage_statement(account, from_ms, to_ms)` (granted to `df_control`): the
requester's own purchases, allocations, charges by line, credits, refunds and balances as
`dungeonflux.finance.export.usage-statement/1`, built only from the customer view types (18.9), so
an export can never show another payer's amount or a fee, markup, provider or cost.

**Content screens.** Admitted through the fence-free account path (11.1); settled as company loss
`content_screen`.

## 13. Time-meter pricing (plan 4.1, O1 5.2)

### 13.1 Holds before time accrues

Time is not a provider dispatch, but it counts against the ceilings and must be funded before it
accrues (plan 4.2 "reserve capacity for the next platform billing interval"). F's meter worker
keeps each active session's **meter hold** at least `meter.intervals_ahead = 2` intervals of
`meter.interval_seconds = 60` (equal to O1's `segment_roll_seconds`) ahead: each extension is a
reservation of kind `time` with shares from the current epoch (16.11). O1's segment open and roll
transaction reads `finance.meter_funded_until(tenant, campaign, session)` (a lock-free read of a
definer function; no finance row lock is taken inside O1's transaction, so D25 holds); if the
next interval is not covered, O1 opens no metered segment and raises `FundingRequired` (meter
free, plan 4.1). If F's worker falls behind, the table goes free, never unfunded-charged.

### 13.2 Pricing closed segments

- Input per segment from O1: `opened_at`, `closed_at` (database clock) and the session's
  `meter_credits` ranges. O1 closes rolls at proven progress (`GREATEST(opened_at,
  last_progress_at)`, O1 5.2), so `closed_at` **is** the proven bound; F has no `proven_until`
  column and no `unproven_ms`. The uncertain tail after the last proven progress is in no closed
  segment (it stays in the open one until the sweeper or next claim closes it at
  `last_progress_at`); F cannot measure it and does not record it. The company absorbs it (O1 5.2).
- **Priced interval** = `[opened_at, closed_at)` minus credited ranges.
- A segment is priced once its `closed_at` is older than O1's `meter.backdate_window_seconds`
  (proposed 45 s), so no credit can arrive after pricing (O1 question B-2: agreed). The open
  segment is never priced.
- **Rational accumulator** per session: `priced_ms`, `charged_micro_dollars` and
  `forgone_micro_dollars` totals.
  `exact = floor((priced_ms + d) * rate / 3_600_000) - (charged + forgone)`; the charge
  is `min(exact, remaining meter holds)`. Charges never exceed the exact amount, and the remainder
  carries to the next interval. Rate = the pinned version's platform rate.
- **The charge is capped at the holds.** `reserved >= 0` is a database check, so a charge larger
  than the session's remaining meter-hold shares would wedge the pricer on a constraint failure.
  The pricer takes `charge = min(exact, held)` from the meter-hold reservations in order of
  creation (16.12), and books the rest, `exact - charge`, as forgone revenue of class
  `meter_unfunded` in `finance.forgone_revenue` (a report row, not a journal entry: no money
  moved and no cost was incurred). The shortfall is added to the `forgone` total so the
  cumulative floor does not charge it again later. `meter_unfunded` appears in the company-loss
  export as forgone revenue beside, not inside, the supplier-cost loss classes (23.2). With
  holds kept two intervals ahead it arises only when the pot ran dry between extensions.
- Idempotent per (session, segment sequence) (TIME-03).
- Durations come only from O1's database-clock timestamps (`clock_timestamp()`); a negative or
  overlapping interval cannot exist (O1's checks) and the pricer refuses one as a typed
  `MeterRefusal` with an alert, never a negative charge (TIME-03).

### 13.3 Paused, all-away and blocked time

Paused, lobby, preparing, funding-required, players-away, owner-unhealthy, frozen and draining time
has no metered segment (O1 5.1), and blocking waits and presence back-dates arrive as credits
(TIME-01, TIME-02). F prices exactly what O1 marks metered and proven.

### 13.4 Ending a session: `settling`, who calls, and the handshake with O1

Releasing unused holds at session end while the last segments are still unpriced (the back-date
window is 45 s) would leave their charge nothing to draw from. So ending is a state, not an event:

1. **Who calls.** `end_session_funding(session)` is called by O1's lifecycle on the `End`
   transition (the runtime's lifecycle task through F's endpoint, or the control-plane sweeper's
   `abandoned` end through the same function as `df_control`; neither needs a fence, and a
   stopped or frozen placement can still end the session). It is idempotent. As a backstop, F's
   meter worker lists `session_funding` rows with status `open` whose O1 phase is `Settling` or
   `Ended` (`df_runtime.table_session_phase(session)`, a lock-free definer read, request to O1; **an absent session counts as ended**)
   every `meter.settling_poll_seconds` and ends them itself, so a lost call cannot leave a session
   open.
2. **`settling`.** `end_session_funding` sets `session_funding.status = 'settling'`, moves every
   allocation to `closing` (8.2), releases each allocation's **unreserved** allocated amount to
   available at once (16.13), and leaves every reservation, meter holds included, in place. New
   admissions and new meter holds are refused (`SessionNotOpen`).
3. **The finish check** (work item `session_finish`, every 30 s while settling) completes when all
   of these hold: O1 has closed the session's last metered segment (phase `Ended` is not required;
   `df_runtime.session_meter_closed(session)` is true, O1 definer); every closed segment is
   priced (older than the back-date window and priced); every provider attempt of the session has
   a customer state other than `Held`.
4. **The finish transaction** releases the remaining meter-hold remainder (16.6), sweeps every
   allocation (8.2) and closes them, sets the session `closed`, marks the statement final, and
   **calls `df_runtime.settlement_finished(tenant, session)`** in the same transaction (finance
   rows first, then O1's session row, D29). That definer is O1's (request): it applies the
   `SettlementFinished` event to the lifecycle, taking `Settling -> Ended`. It is idempotent,
   takes no fence (settlement needs no owner), and is granted to `df_finance` only. Until it is
   called the statement is provisional while any attempt is unresolved (plan 9.11).
5. Time bound: a session whose finish check has not completed after `session.settling_alert_hours
   = 6` (proposed) raises an alert; nothing is released or forfeited by timeout.

## 14. Refunds, disputes, chargebacks and tax (plan 6.7, 5.5, J13)

### 14.1 Refunds of unspent purchased credit

- `preview_refund(payer)`: eligible = purchased lots in state `Active`, available balance, plus the
  unreserved part of the payer's allocations if the payer chooses to release them (J13); never
  promotional credit, frozen, disputed or held amounts. Lots newest first. Each lot rounds down to
  whole cents. The preview lists lots, amounts, released allocations, pending holds, tax reversal
  and the destination card suffix (from B's checkout record). A lot older than
  `refund.processor_refund_window_days = 180` (proposed; card networks limit refund age) is listed
  as **manual payout** (below).
- `request_refund(plan, operation)`: one transaction locks the payer row, the sessions and
  allocations being released, and the lots; moves the amount to `refund_payable` (16.9); writes
  `refund`, `refund_lot` and the `refund_outbox` rows for B's processor worker. Idempotency key to
  the processor = `refund_lot_id` (one processor refund per original charge). A refund lot with an
  open dispute on its lot is `blocked_by_dispute` (14.2) and the outbox does not offer it.
- B reports `observe_refund_lot(refund_lot, KnownSuccess | KnownFailure | Unknown)`:
  success posts cash out (16.9); failure moves the refund lot to `failed_review` (plan 6.7: "not
  immediately to both cash and credit") and an operator either retries, pays it manually, or
  returns it to credit with a reason (return follows the lot-state rule of 14.4); unknown keeps it
  payable and B reconciles with the same key (REF-02).
- **Manual payout (ruling (g)).** A lot past the processor window, or a refund the processor
  rejected for age, is paid by the operator through another method outside the system. The refund
  lot is created as `manual_pending` (the cash is `refund_payable` from the request, so the
  customer's credit is already out of the spendable balances); B's worker never claims it. When
  the money has left the bank the operator, with permission `ManualRefundPayout` and a case
  reference, records it, which posts the dedicated entry kind `manual_refund_payout` (16.19): Dr
  `refund_payable{lot}` / Cr `bank_operating`, with the bank transaction reference as the
  immutable external reference. The kind needs an approver other than the actor above the
  adjustment threshold (17.6). F stores no bank account or payment details; the operator holds
  them out of band.
- Processor fees are not returned on refunds (plan 6.7, S16): the fee expense stays.
- `refund.unused_credit_window_days = none` (proposed: no expiry fee or window). Both windows are
  owner questions (28.3).

### 14.2 Disputes and chargebacks (plan 5.5, REF-03)

The per-state posting rules for anything that comes back to a lot are in 14.4; this section is the
lifecycle of the lot itself.

- **Opened** (B reports the dispute state for the payment): the lot becomes `Frozen`. Its
  available balance and the unreserved allocated portions of that lot move to `purchased_frozen`
  (16.10); other lots and other payers are untouched. **Reserved portions stay reserved:** in-flight
  holds settle normally (sent work stays attached), and the charged part of such a settlement is
  ordinary revenue; its surplus returns to `purchased_frozen`, not to an allocation (14.4). A
  **pending `refund_payable`** of that lot stays payable but its refund lot becomes
  `blocked_by_dispute`: B's worker does not start new processor refunds on a disputed charge
  (calls already in flight resolve normally). The processor's withdrawal of the disputed amount and
  its dispute fee are posted.
- **Won:** funds return; frozen returns to available (allocations are not restored; the payer can
  allocate again); blocked refund lots are released to B's outbox; the lot becomes `Active`.
- **Lost, for `D` micro-dollars** (the amount the processor takes back; usually the whole charge,
  possibly less). One entry, `dispute_lost` (16.10), **extinguishes in a fixed order and never posts
  a negative amount**. With `f` the frozen balance, `p` the pending refund payable on the lot and
  `r` the lot's reserved portion:
  1. *frozen first*: `x1 = min(f, D)` of `purchased_frozen` is cleared against the withheld funds;
  2. *then the refund liabilities that no processor call is in flight for*: with `D1 = D - x1`,
     refund lots are extinguished whole, newest first, while `D1` still covers each one's
     principal (`x2` = the sum). The `refund_lot` states (17.4: `pending`, `unknown`, `paid`,
     `failed_review`, `returned_to_credit`, `manual_pending`, `blocked_by_dispute`,
     `cancelled_dispute_lost`) are treated exactly:
     - **extinguished** (their `2500` liability is cleared, the lot becomes
       `cancelled_dispute_lost`): `pending`, `failed_review`, `manual_pending` and
       `blocked_by_dispute`. A `manual_pending` lot is a payout the operator has not made yet and a
       `blocked_by_dispute` lot is a pending refund the dispute froze; the bank has taken the money
       back, so neither is owed any longer, as with the other pending liabilities;
     - **never extinguished, so `x2 = 0` for them**: `unknown` (a processor call is in flight; if the
       processor then pays it, the liability must still be there), and the final states `paid`,
       `returned_to_credit` and `cancelled_dispute_lost`, which hold no liability;
     - a lot the remainder `D1` cannot cover whole stays in its state and stays payable;
  3. *then loss*: `x3 = D1 - x2 >= 0` is booked as `chargeback_loss`; by construction it is never
     negative, whatever `D`, `f` and `p` are;
  4. *the rest of the frozen balance* `f - x1` (when `D < f`) is **released** by a `2110` credit line
     in the same entry (the `dispute_release` part of `dispute_lost`): it returns to
     `purchased_available` and stays spendable. The entry balances because
     `x1 + x2 + x3 = D` and `(f - x1) + x1 = f`.
  The processor's withheld amount `D` is credited to `dispute_funds_withheld`.
  **Lot state afterwards is a function of the balances.** If the lot's backing is gone, that is
  `D` covers the whole unrefunded amount of the lot (`original - refunded - charged_back = D`), the
  lot becomes `ChargedBack` ("no spendable remainder, nothing backs any liability still on it").
  The lot's still-reserved portion `r` is then unfunded: the loss `x3` includes it, and it stays
  reserved until its reservation resolves, when it is either charged (revenue for service actually
  delivered) or released as a chargeback recovery (Dr reserved / Cr `chargeback_loss`). Over the
  life of the lot the loss nets to what was consumed and not paid for. If cash still backs the
  lot (a **partial** loss), the lot stays `Active` with `charged_back_micro_dollars` raised by
  `D`, the released remainder is spendable, and reserved portions resolve by the ordinary rules
  (surplus returns to the allocation), because they are funded; refund lots left pending are no
  longer blocked and go back to B's outbox.
- Never debit the host or another contributor (plan 5.5, REF-03): the posting constructors take
  only that lot's keys.
- B decides account standing (restricted) and calls `set_payer_standing` in that transaction
  (19.4).

### 14.3 Tax fields (plan 6.7, owner gate)

- `tax.mode = none` (proposed default: US pilot, tax treatment pending a specialist). Modes
  `at_purchase` and `at_consumption` have postings ready (16.2, 16.5) but stay off.
- Each lot stores its tax evidence: billing country and region from the processor, the tax policy
  version, the mode, jurisdiction, rate as a ratio, and the tax amount at purchase. Different
  contributors may differ (plan 6.7); the pot shows usable service balance and maximum bank charge
  separately.
- Refunds reverse tax in proportion to the refunded principal of each lot. Tax is never charged
  twice: a lot taxed at purchase is consumed without consumption tax.
- `tax.jurisdictions = []`: launch availability limited to reviewed jurisdictions (owner).

### 14.4 Where credit goes when it comes back: the return rule per lot state

Every flow that returns credit to a lot (a release, a settlement surplus, a meter-hold remainder, a
customer credit, a refund returned to credit, an allocation release) asks one pure function,
`return_target(lot_state, allocation_status)`, and posts to the key it answers. No posting
constructor chooses the key itself.

| Lot state | Returned from a hold or surplus | Customer credit (reversal of a charge) | Refund returned to credit |
|---|---|---|---|
| `Active` | allocation `active` or `consent_incompatible`: `*_allocated{allocation, lot}`; any other allocation status, or no allocation: `*_available{payer, lot}` | `*_available{payer, lot}` | `*_available{payer, lot}` |
| `Frozen` (dispute open) | `purchased_frozen{lot}` in every case; it follows the dispute: won -> available, lost -> cleared (14.2) | `purchased_frozen{lot}` | `purchased_frozen{lot}` |
| `ChargedBack` (a dispute lost for the whole unrefunded amount; 14.2) | a **chargeback recovery**: Dr `purchased_reserved` / Cr `chargeback_loss{processor}`; never spendable | refused `CreditTargetChargedBack`; the operator may grant goodwill (promotional) instead | cannot occur: the pending refund is extinguished at the loss |
| `Closed` | cannot occur for a hold (a lot with an open hold is not closed); a credit reopens the lot (`Closed -> Active`, under the payer lock) | reopen, then as `Active` | reopen, then as `Active` |
| promotional `Active` | as `Active`, in the Promo book | `promotional_available` (and gross-mode reversal) | n/a (promotional credit is never refunded) |
| promotional `Forfeited` | cannot occur: closure requires no open holds (14.5) | refused | n/a |

Lots are closed only by a payer-locked job (`close_settled_lots(payer)`, 19.1): a lot is `Closed`
when every key of it is zero, it has no open reservation share and no pending refund. Reopening
happens in the credit and refund-return flows, which hold the payer lock. A deferred trigger checks
every state change against the lot's balances (17.3).

**External refund drain order** (a dashboard refund `R` the processor reports that is not one of our
refund lots, 16.14). The recorded lot balances are reduced in this order, each step taking what it
can: (1) `purchased_available`; (2) the unreserved `purchased_allocated` of the lot's allocations
(the allocation's remaining authorization shrinks, the payer is told); (3) `purchased_reserved`
(newest hold first; the hold shrinks, its settlement charges at most what remains, and the
uncharged part of a delivered result is recorded as forgone revenue `refunded_externally` in
`finance.forgone_revenue` rows, never charged to another payer); (4) the remainder, which was consumed, to `refunds_of_consumed_service` (contra
revenue). A pending own `refund_payable` is not drained: the processor will refuse our refund if
the charge has too little left, the refund lot then goes to `failed_review` (14.1), and the conservation
equation (21.7) still holds because the external refund took the consumed part. A `Frozen` lot is
drained from `purchased_frozen` first. The sum of the four steps is `R`.

### 14.5 Account closure (plan 9.9, B's closure path)

`closure_blockers(account)` reports: purchased credit of at least one cent (10,000 micro-dollars)
in available, allocated or frozen; open holds; pending refund lots; open disputes. **A sub-cent
purchased remainder does not block closure.** B's closure completion transaction then calls, in this
order, `release_account_allocations(account)`, `round_closing_remainder(account)` (one `rounding`
entry per lot, 16.9, to the single rounding account) and `forfeit_promotional(account)`: promotional
balances are forfeited at closure (ruling (g)), one `promotional_forfeit` entry per promotional lot
(16.18) clearing the Promo book against `promotional_offset`, lot state `Forfeited`. The
forfeiture is reported to the customer at the closure confirmation. Closure cancels the account's
outgoing transfer proposals (D31, S1's side).

## 15. Company-loss classes, unpriced routes and the tariff

### 15.1 Classes of `absorbed_supplier_cost` (5200), closed enum `LossClass`

| Class | When |
|---|---|
| `failed_no_output` | sent, billed, no usable output (D21), including output lost after provider success and O2's `output_lost`, `staged_lost`, `staged_expired`, `supplier_failed`, `screen_unavailable` |
| `invalid_output` | output invalid or rejected by validation or judgement (O2 `Invalid(...)`) |
| `duplicate` | a duplicate result that was not the delivered one |
| `superseded` | output superseded by a replacement attempt (O2 F01) |
| `platform_defect` | our defect; also the reclass on a `TechnicalFailure` credit |
| `speculative_unused` | company-admitted speculative work never used (plan 2.2, 6.3) |
| `unknown_outcome` | an unknown outcome later found billed, or written off at the worst case |
| `late_result_after_release` | billed result after the customer hold was released (JOB-05) |
| `overrun_beyond_quote` | supplier cost above the supplier maximum |
| `usability_unreported` | non-media output with no usability report 300 s after the result (media never uses this class, 12.1) |
| `content_screen` | content screens of uploads (D13), a company safety cost; includes screens of uploads that turn out to be rejected (ruling (d)) |
| `company_internal` | moderation, quality checks, memory maintenance and other company-paid work |
| `unpriced_route` | a route the price version cannot quote (15.2) |
| `unmetered_route` | spend found by invoice reconciliation that no attempt explains (a `Pending` route or a leak); always an alert |

**Forgone revenue** is not a supplier cost and is never in 5200. It is a report table
(`finance.forgone_revenue`, no journal entry) with a closed reason: `meter_unfunded` (time that
was provided but could not be charged because the holds were exhausted, 13.2) and
`refunded_externally` (a delivered charge reduced by an external refund that drained its hold,
14.4). The uncertain meter tail (`uncertain_meter_tail`) of round 0 is removed: F cannot measure
it (13.2).

### 15.2 Route status, unpriced and unmetered routes (ruling (e), H6)

- `Metering::Pending` routes are refused in hosted paid mode (O1 10, B 8.8, ruling (e)): no
  attempt, no exposure, refusal `RouteUnmetered`. A route leaves that state only when C meters it
  and its categories have tariff rows. H6 counts nine of fifteen routes as Pending. **The effect at
  launch is stated to the owner** so it is a decision, not a surprise:

  | H6 route | Status | Hosted effect until C meters it |
  |---|---|---|
  | R1, R2 picture edit (OpenAI), R3 BFL picture, R5 `gpt-6-luna` text, R6 ElevenLabs speech | Priced | admitted; R1-R3 need a `calls` row or the unknown-outcome estimate is zero (H6 U1) |
  | R4 fal composite picture | Unpriced | company absorb (below) |
  | R7 speech-to-text (Scribe) | Pending | refused: no microphone transcription |
  | R8 embeddings | Pending | refused: no story-memory embeddings |
  | R9 SAM segmentation | Pending | refused: no figure masks or plate segments |
  | R10, R11 `gpt-5.5` head locator and plate judge | Pending, `image_input` row missing | refused |
  | R12 music | Pending | refused: no place music |
  | R13 video (Kling) | Pending | refused: no cinematics |
  | R14 splat worlds | Pending | refused |
  | R15 Jev (TypeSafe) | Pending, reports no usage | **refused until C meters it; Jev is metered under D21, so C closes this route first** (C asks 4 and 7) |

- `Metering::Unpriced` routes (today the fal composite): `unpriced_route_policy = absorb`
  (proposed): admitted only as `Payer::Company(unpriced_route)` with the per-route supplier
  maximum from `content/finance.json` (`unpriced_route_supplier_maximum.<route>`); never charged
  to a customer. `refuse` is the alternative. A customer-paid admission on an unpriced category is
  always `CategoryUnpriced` (9.2).
- The forgone-revenue rows are not a supplier cost.

### 15.3 What a usage report may contain (H6 5, 6)

`record_usage(attempt, SupplierOutcome, Usability)` and the dispatcher's `record_transport_outcome` carry exact integer units per category and
nothing in dollars that F relies on. The type has no float field; an optional
`reported_cost_evidence_micro_dollars` is stored for comparison only. Rules F enforces:

- Units come from the pinned table's categories; a category the route did not declare, or the
  price version cannot price, is rejected.
- A report with no units (dollars only) is `UsageWithoutUnits` and is held as `Unknown`; the
  attempt keeps its worst-case exposure until reconciliation.
- The hard-coded and float price paths of H6 5 and 6 (the picture price, the SAM price, the live
  Kling per-second price, the registry's published rates) do not reach F: their callers must report
  units, and a unit report derived from such a path is stored as `source = Estimated`, priced from
  the pinned row. A float-priced charge cannot exist (asked of C, 29.3).
- Unit gaps (H6 U1-U4, U6): the unknown-outcome estimate for a picture is the route's
  `maximum_units` per category (calls are priced if the tariff has a `calls` row, else the
  picture's token maxima), never one `Calls` unit at zero; the unknown-outcome text estimate prices
  input and output tokens at their own rates.

## 16. Journal postings per flow

Amounts follow the plan's examples. G = General book, P = Promo book, X = Exposure book. "Dr/Cr" use
the account names of section 5 with their dimensions in braces. Every entry has a kind (a member
of the allowlist of 17.6), a business key (section 20), an external reference where the kind
requires one, correlation columns and a reason code. Where credit returns to a lot, the target key
is the one `return_target` names (14.4); the examples show the `Active` case.

**16.1 Trial grant ($10.00).** `trial_grant`.
P Dr `promotional_offset{trial}` 10.00 / Cr `promotional_available{payer, lotT}` 10.00.

**16.2 Purchase confirmed ($20.00, no tax).** `purchase`.
G Dr `processor_clearing{stripe}` 20.00 / Cr `purchased_available{payer, lot1}` 20.00.
With tax at purchase (off): Dr clearing 21.20 / Cr `purchased_available` 20.00, Cr `tax_payable{j}` 1.20.
Fee, when known (`processing_fee`): G Dr `processing_fees{stripe}` 0.88 / Cr clearing 0.88.
Payout (`payout`): G Dr `bank_operating` 19.12 / Cr clearing 19.12. (Plan 6.5 rows 1-3.)

**16.3 Contribution and allocation ($15.00 from a payer holding $10 trial and $20 purchased).**
`allocation`. Draw order: promotional first.
P Dr `promotional_available{payer, lotT}` 10.00 / Cr `promotional_allocated{alloc, lotT}` 10.00.
G Dr `purchased_available{payer, lot1}` 5.00 / Cr `purchased_allocated{alloc, lot1}` 5.00.
A contribution (J03) is a purchase (16.2) if needed, then this entry; it changes the pot only after
the purchase is confirmed.

**16.4 Reservation at admission (retail maximum $0.90 split 50/25/25 over allocations A, B, C;
supplier maximum $0.60).** `reservation`.
Shares 0.45 / 0.225 / 0.225. For each share, its lot portions in draw order, e.g. A from lotT:
P Dr `promotional_allocated{A, lotT}` 0.45 / Cr `promotional_reserved{A, lotT}` 0.45;
G Dr `purchased_allocated{B, lot2}` 0.225 / Cr `purchased_reserved{B, lot2}` 0.225; same for C.
Exposure: X Dr `supplier_exposure_open{openai}` 0.60 / Cr `supplier_exposure_offset{openai}` 0.60.
(Each book balances on its own; a line cannot move value from one book to another.) A company
admission posts only the exposure lines.

**16.5 Settlement, known and usable (actual retail $0.72, supplier $0.48).** `settlement`.
Charge split over held shares: A 0.36, B 0.18, C 0.18; each held share is cleared in one line
and its credit goes to the charge and the surplus release (one line per key per entry, 17.1).
P Dr `promotional_reserved{A, lotT}` 0.45 / Cr `promotional_offset{trial}` 0.36 (net mode, the
default), Cr `promotional_allocated{A, lotT}` 0.09 (surplus; `promotional_available` if A is no longer
active, 14.4).
Gross mode adds G Dr `promotional_credit_expense{trial}` 0.36 / Cr `promotional_revenue{trial}` 0.36.
G Dr `purchased_reserved{B, lot2}` 0.225, Dr `purchased_reserved{C, lot3}` 0.225 / Cr
`revenue_ai_usage` 0.36, Cr `purchased_allocated{B, lot2}` 0.045, Cr `purchased_allocated{C, lot3}` 0.045.
Supplier: X Dr `supplier_exposure_offset{openai}` 0.60 / Cr `supplier_exposure_open{openai}` 0.60;
G Dr `cost_of_services{openai}` 0.24, Dr `cost_of_services_promotional{openai}` 0.24 / Cr
`supplier_payable{openai}` 0.48 (the 0.48 split by `split_exactly` over the charge's purchased
0.36 and promotional 0.36 parts, so 0.24 and 0.24; with an odd micro-dollar the tie goes to
purchased).
(Postpaid provider; a prepaid provider credits `supplier_prepaid{p}` instead, per configuration.)
With tax at consumption (off): the share also credits `tax_payable{j}`.

**16.6 Release (cancel before dispatch, a deadline cancel, `VerifiedUnsent`, or the customer hold
released at the deadline).** `release`.
Each held share: Dr `*_reserved{alloc, lot}` / Cr the `return_target` key: `*_allocated{alloc,
lot}` while the allocation is active, `*_available{payer, lot}` once it is withdrawn, departed,
closing or closed, `purchased_frozen{lot}` for a Frozen lot, and for a ChargedBack lot the
chargeback recovery Dr `purchased_reserved{alloc, lot}` / Cr `chargeback_loss{stripe}`. Before
dispatch (and for `VerifiedUnsent`, `CancelledUnsent`) also close the exposure (X Dr offset / Cr
open). After dispatch the exposure stays. Whenever this entry resolves the last open reservation of
a non-active allocation, `sweep_allocation` (8.2) adds an `allocation_release` in the same
transaction.

**16.7 Company loss (sent, billed $0.48, no usable output; D21).** `company_loss`.
Customer side: 16.6. Supplier: X Dr offset 0.60 / Cr open 0.60;
G Dr `absorbed_supplier_cost{openai, failed_no_output}` 0.48 / Cr `supplier_payable{openai}` 0.48.
Content screen, late result, unknown found billed: the same with their class.
Unknown written off (`supplier_exposure_write_off`): G Dr `absorbed_supplier_cost{p, unknown_outcome}`
worst case / Cr `supplier_accrued_unconfirmed{p}`, X Dr offset / Cr open; matched later
(`supplier_invoice_match`): Dr accrued / Cr payable (confirmed), the rest reversed Dr accrued /
Cr absorbed, and a difference inside the tolerance Dr/Cr `rounding` (4.9).

**16.8 Customer credit after a charge (reverse $0.18 charged to B).** `customer_credit`.
G Dr `revenue_ai_usage` 0.18 / Cr `purchased_available{payerB, lot2}` 0.18 (to `purchased_frozen`
for a Frozen lot, refused for a ChargedBack lot; promotional part:
P Dr `promotional_offset` / Cr `promotional_available`; gross mode also reverses 5300/4200).
`TechnicalFailure` adds the cumulative reclassification of 12.5: G Dr
`absorbed_supplier_cost{p, platform_defect}` / Cr `cost_of_services{p}` and Cr
`cost_of_services_promotional{p}` for the matching cost, in proportion to the reversal.
Goodwill (`goodwill_grant`): P Dr `promotional_offset{goodwill}` / Cr `promotional_available{payer, lotG}`.

**16.9 Refund ($12.40 unspent across lots 2 and 1, newest first; REF-01).** `refund_requested`.
G Dr `purchased_available{payer, lot2}` 10.00, Dr `purchased_available{payer, lot1}` 2.40 /
Cr `refund_payable{lot2}` 10.00, Cr `refund_payable{lot1}` 2.40.
Per lot on success (`refund_paid`): G Dr `refund_payable{lot2}` 10.00 / Cr `processor_clearing` 10.00
(with purchase tax: also Dr `tax_payable{j}` / Cr clearing for the proportional tax).
Failure, operator returns it to credit (`refund_returned_to_credit`): Dr `refund_payable` / Cr
`purchased_available` (Frozen lot: `purchased_frozen`).
Unknown: no entry. Closure rounding (`rounding`): G Dr `purchased_available{payer, lot}` 0.004 /
Cr `rounding` 0.004.

**16.10 Dispute ($20 lot: $8 available, $4 allocated unreserved, $0.90 reserved, $7.10 consumed;
$5 of the available is already refund payable in the second example).**
Open (`dispute_freeze`): G Dr `purchased_available{payer, lot}` 8, Dr
`purchased_allocated{alloc, lot}` 4 / Cr `purchased_frozen{lot}` 12. The $0.90 stays in
`purchased_reserved`.
Withdrawal (`dispute_withheld`): G Dr `dispute_funds_withheld{stripe}` 20 / Cr clearing 20;
dispute fee Dr `processing_fees{stripe}` 15 / Cr clearing 15.
Won (`dispute_won`): Dr clearing 20 / Cr withheld 20; Dr `purchased_frozen{lot}` 12 / Cr
`purchased_available{payer, lot}` 12 (a returned fee: Dr clearing / Cr `processing_fees`).
Lost, whole charge, D = 20 (`dispute_lost`, the order of 14.2: frozen, then pending refund, then
loss; the loss is never negative): Dr `purchased_frozen{lot}` 12, Dr `chargeback_loss{stripe}` 8 / Cr
withheld 20 (x1 = 12, x2 = 0, x3 = 8). With $5 refund payable pending at the loss the frozen
balance is 7: Dr frozen 7, Dr `refund_payable{lot}` 5, Dr `chargeback_loss` 8 / Cr withheld 20
(x1 = 7, x2 = 5, x3 = 8). The lot becomes `ChargedBack`.
Lost, **partial**, D = 5 on the 12 frozen (D < frozen): Dr `purchased_frozen{lot}` 12 / Cr withheld 5,
Cr `purchased_available{payer, lot}` 7 (the released remainder; x1 = 5, x2 = 0, x3 = 0, so no loss).
The lot stays `Active` with `charged_back` raised by 5 and the 7 spendable.
Lost, D = 15 on frozen 12 and no refund: Dr frozen 12, Dr `chargeback_loss` 3 / Cr withheld 15, no
release line; the lot stays `Active` if cash still backs it, else `ChargedBack`.
Later, the held $0.90 resolves: charged 0.72 -> Dr `purchased_reserved{alloc, lot}` 0.72 / Cr
`revenue_ai_usage` 0.72; surplus 0.18 -> `chargeback_recovery`: Dr `purchased_reserved{alloc, lot}`
0.18 / Cr `chargeback_loss` 0.18. Net loss over the life of the lot: 7.82, equal to the revenue
recognized on it.

**16.11 Meter hold (one 60 s interval at $2/h; a hold is a maximum, so $0.0333333 rounds up to
$0.033334; split 2/3 and 1/3 by largest remainder).** `meter_hold`.
G Dr `purchased_allocated{A, lot}` 0.022223 / Cr `purchased_reserved{A, lot}` 0.022223; B 0.011111.
(Promotional portions in P as in 16.4.) No exposure lines.

**16.12 Meter charge (a priced interval of 42 s: cumulative rule gives $0.023333).** `meter_charge`.
G Dr `purchased_reserved{A, lot}` 0.015556, Dr `purchased_reserved{B, lot}` 0.007777 / Cr
`revenue_table_time` 0.023333 (largest remainder over the hold's shares; promotional portions in P).
When the holds cannot cover the exact amount the charge is capped at the holds and the shortfall is a
`forgone_revenue` row, not an entry (13.2). The remainder of the meter holds is released at the
session finish by a `release` entry (13.4).

**16.13 Withdrawal, departure and session end.** `allocation_release`.
Each unreserved allocated portion: Dr `*_allocated{alloc, lot}` / Cr `*_available{payer, lot}`
(plan 5.4: back to each contributor, not the host). A Frozen lot has no allocated portion (it moved
to frozen at the dispute).

**16.14 External refund (dashboard refund of $5 on a lot with $3 unspent).** `external_refund`.
G Dr `purchased_available{payer, lot}` 3, Dr `refunds_of_consumed_service` 2 / Cr clearing 5.
Drain order (14.4): available, unreserved allocated, reserved, then consumed.

**16.15 Promotional expiry (only if `promotional.expiry_days` is ever set).** `promotional_expiry`.
P Dr `promotional_available{payer, lot}` x / Cr `promotional_offset{source}` x.

**16.16 Supplier invoice and payment.** `supplier_payment`: G Dr `supplier_payable{p}` / Cr
`bank_operating`. `supplier_prepayment`: G Dr `supplier_prepaid{p}` / Cr `bank_operating`.
`supplier_invoice_match`: 16.7.

**16.17 Manual adjustment and reversal.** `manual_adjustment`: a balanced entry over the
**listed** accounts only (17.6: 2300, 2310, 2400, 3000, 4100, 4110, 4190, 5100, 5110, 5200, 5400,
5500, 5700 and nothing in 1xxx, 21xx, 2500, 22xx, 4200, 5300 or the Exposure book), with reason,
linked support case, actor and an approver who is not the actor above the configured threshold
(plan 7.3). Customer credit is never changed by a manual adjustment: goodwill is its own kind
(16.8). `reversal`: the exact negation of an earlier entry of a **reversible** kind (those that
carry no subledger change: `manual_adjustment`, `processing_fee`, `payout`, `supplier_payment`,
`supplier_prepayment`, `supplier_invoice_match`, `supplier_exposure_write_off`, `rounding`),
referencing it (`reverses_entry_id`, unique, with a trigger enforcing exact negation); history is
never edited (plan 6.5). A mistake in a customer-affecting entry is corrected by that flow's own
inverse (credit, refund, release), never by a raw reversal, so the subledgers cannot drift. There is
no "set balance" operation (plan 9.13).

**16.18 Promotional forfeiture at closure (ruling (g)).** `promotional_forfeit`.
P Dr `promotional_available{payer, lot}` x / Cr `promotional_offset{source}` x, per promotional lot,
at account closure. Closure requires no allocated or reserved promotional balance (14.5). In net
and gross mode alike nothing is posted in the General book: the promotional expense (5300) is
recognized at consumption in gross mode, so the forfeiture reverses the grant side of the
memorandum, the "promotional expense reversal" of the ruling, and the lot becomes `Forfeited`.

**16.19 Manual refund payout (ruling (g)).** `manual_refund_payout`.
G Dr `refund_payable{lot}` 10.00 (with purchase tax: Dr `tax_payable{j}` proportionally) / Cr
`bank_operating` (10.00 + tax). External reference: the bank transaction. Anchor: the refund lot in
`manual_pending`. Actor operator, case reference required, approver distinct from the actor above
the threshold.

## 17. PostgreSQL schema and migrations 0020-0024

Schema `finance` in the one hosted database (D7), PostgreSQL 17 or later (target 18; D30). Conventions
follow B's 0005: 16-byte `bytea` ids with length checks, closed enums as `text CHECK (... IN
...)`, parameterized SQL only, append-only tables refuse `UPDATE`/`DELETE`/`TRUNCATE` by trigger and
have those privileges revoked, each migration is one transaction in O1's migration ledger. **Times:**
`recorded_at_ms` and every `*_at_ms` that orders ledger events are read from the database clock
inside the definer function (`clock_timestamp()`), never passed in by the service; service-supplied
times (an event's `effective_at_ms`) are information only (17.8). **Isolation:** every finance
transaction runs at `READ COMMITTED` with explicit row locks (19.1); correctness comes from the
locks and constraints, not from `SERIALIZABLE`.

### 17.1 `0020_finance_journal.sql`: the journal and its constraints

```sql
CREATE TABLE finance.ledger_account (
  code smallint PRIMARY KEY,
  name text NOT NULL UNIQUE,
  book text NOT NULL CHECK (book IN ('general', 'promo', 'exposure')),
  kind text NOT NULL CHECK (kind IN ('asset', 'liability', 'equity', 'revenue',
                                     'contra_revenue', 'expense', 'memo')),
  normal_side smallint NOT NULL CHECK (normal_side IN (1, -1)),  -- 1 debit-normal, -1 credit-normal
  balance_enforced boolean NOT NULL,
  dimensions text[] NOT NULL);           -- the dimension columns a key of this code must set
-- seeded with section 5; the seed is the only write; later migrations add rows, never change them.

CREATE TABLE finance.account_key (
  key_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  code smallint NOT NULL REFERENCES finance.ledger_account,
  currency char(3) NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
  payer_account_id bytea,
  lot_id bytea,            -- FK finance.funding_lot, added in 0021
  allocation_id bytea,     -- FK finance.allocation, added in 0021
  provider text, processor text, loss_class text, jurisdiction text, promo_source text,
  UNIQUE NULLS NOT DISTINCT (code, currency, payer_account_id, lot_id, allocation_id,
                             provider, processor, loss_class, jurisdiction, promo_source),
  CHECK (finance.key_shape_is_valid(code, payer_account_id, lot_id, allocation_id, provider,
                                    processor, loss_class, jurisdiction, promo_source)));

CREATE TABLE finance.account_balance (                  -- only for balance_enforced codes
  key_id bigint PRIMARY KEY REFERENCES finance.account_key,
  natural_micro_dollars bigint NOT NULL CHECK (natural_micro_dollars >= 0));

CREATE TABLE finance.ledger_stream (                    -- one chain head per lock anchor (17.7)
  stream_kind text NOT NULL CHECK (stream_kind IN ('payer', 'session', 'campaign', 'account', 'platform')),
  stream_id bytea NOT NULL,                             -- account, session or campaign id; 16 zero bytes for platform
  last_seq bigint NOT NULL, last_hash bytea NOT NULL CHECK (octet_length(last_hash) = 32),
  PRIMARY KEY (stream_kind, stream_id));

CREATE TABLE finance.journal_entry (
  entry_id bytea PRIMARY KEY CHECK (octet_length(entry_id) = 16),
  kind text NOT NULL REFERENCES finance.entry_kind,    -- the closed allowlist of 17.6, not a CHECK list
  business_key bytea NOT NULL CHECK (octet_length(business_key) = 32),  -- section 20
  external_ref_kind text NOT NULL CHECK (external_ref_kind IN ('none', 'processor_payment',
    'processor_refund', 'processor_dispute', 'processor_balance_txn', 'processor_payout',
    'supplier_invoice', 'bank_transaction', 'support_case', 'attempt', 'trial_grant')),
  external_ref text CHECK (external_ref IS NULL OR octet_length(external_ref) <= 128),
  stream_kind text NOT NULL, stream_id bytea NOT NULL,
  stream_seq bigint NOT NULL CHECK (stream_seq >= 1),   -- gapless per stream
  global_seq bigint NOT NULL UNIQUE DEFAULT nextval('finance.global_seq'),  -- display and export order only; NOT commit-ordered, never a watermark or a replay position (positions are per stream)
  prev_hash bytea NOT NULL CHECK (octet_length(prev_hash) = 32),
  entry_hash bytea NOT NULL CHECK (octet_length(entry_hash) = 32),
  entry_hash_version smallint NOT NULL CHECK (entry_hash_version >= 1),  -- which frozen encoding produced entry_hash (17.6)
  effective_at_ms bigint NOT NULL,                      -- business time, information only
  recorded_at_ms bigint NOT NULL,                       -- database clock
  period_id integer NOT NULL REFERENCES finance.accounting_period,  -- derived by post_entry (17.8)
  line_count smallint NOT NULL CHECK (line_count BETWEEN 2 AND 64),
  reverses_entry_id bytea REFERENCES finance.journal_entry,
  actor_kind text NOT NULL CHECK (actor_kind IN ('system', 'operator')),
  actor_id bytea, approver_id bytea,
  reason text NOT NULL,                  -- CHECK over the closed EntryReason codes, listed in round 2
  case_ref text CHECK (octet_length(case_ref) <= 64),
  event_id bytea NOT NULL,               -- the finance event (17.6) that carried this entry
  campaign_id bytea, session_id bytea, attempt_id bytea, reservation_id bytea,
  price_version_id bytea,
  corrects_period_id integer REFERENCES finance.accounting_period,  -- set when a closed period is corrected in the open one (17.8)
  UNIQUE (kind, business_key),
  UNIQUE (stream_kind, stream_id, stream_seq),
  UNIQUE (reverses_entry_id),                           -- one reversal per entry
  CHECK ((kind = 'reversal') = (reverses_entry_id IS NOT NULL)),
  CHECK (actor_kind = 'system' OR (actor_id IS NOT NULL AND case_ref IS NOT NULL)),
  CHECK (approver_id IS NULL OR actor_id IS NULL OR approver_id <> actor_id));

CREATE TABLE finance.journal_line (
  entry_id bytea NOT NULL REFERENCES finance.journal_entry,
  line_no smallint NOT NULL CHECK (line_no >= 1),
  key_id bigint NOT NULL REFERENCES finance.account_key,
  amount_micro_dollars bigint NOT NULL CHECK (amount_micro_dollars <> 0),  -- + debit, - credit
  PRIMARY KEY (entry_id, line_no),
  UNIQUE (entry_id, key_id));
CREATE INDEX journal_line_key ON finance.journal_line (key_id);
```

Supporting tables of 0020 (the ones the text of 17.6-17.8 relies on):

```sql
CREATE TABLE finance.accounting_period (
  period_id integer PRIMARY KEY CHECK (period_id BETWEEN 200001 AND 999912),   -- yyyymm
  starts_at_ms bigint NOT NULL, ends_at_ms bigint NOT NULL CHECK (ends_at_ms > starts_at_ms),
  status text NOT NULL CHECK (status IN ('open', 'closed')),
  closed_at_ms bigint, closed_by bytea, reviewed_by bytea,
  digest bytea CHECK (digest IS NULL OR octet_length(digest) = 32),
  CHECK ((status = 'closed') = (closed_at_ms IS NOT NULL AND digest IS NOT NULL)),
  CHECK (closed_by IS NULL OR reviewed_by IS NULL OR closed_by <> reviewed_by));

CREATE TABLE finance.accounting_time_zone_history (    -- 17.8: changes apply from a future instant only
  effective_from_ms bigint PRIMARY KEY,
  time_zone text NOT NULL,                              -- an IANA name; 'UTC' at launch
  recorded_at_ms bigint NOT NULL);

CREATE TABLE finance.entry_kind (
  kind text PRIMARY KEY,
  stream_rule text NOT NULL CHECK (stream_rule IN ('payer', 'session', 'campaign', 'account', 'platform', 'original')),
  anchor text NOT NULL, external_ref_kind text NOT NULL,
  actors text[] NOT NULL, reversible boolean NOT NULL,
  needs_approver_above_micro_dollars bigint);
CREATE TABLE finance.entry_kind_rule (                  -- the allowlist of 17.6
  kind text NOT NULL REFERENCES finance.entry_kind,
  code smallint NOT NULL REFERENCES finance.ledger_account,
  side smallint NOT NULL CHECK (side IN (1, -1)),
  PRIMARY KEY (kind, code, side));
CREATE TABLE finance.event_kind (                       -- every kind of journaled event (17.6)
  event_kind text PRIMARY KEY, carries_entries boolean NOT NULL, journaled boolean NOT NULL CHECK (journaled));
CREATE TABLE finance.event_marker (                     -- one row per committed event; the write-guard reads it
  txid bigint NOT NULL, event_id bytea NOT NULL, event_kind text NOT NULL REFERENCES finance.event_kind,
  PRIMARY KEY (txid, event_id));
CREATE TABLE finance.replay_state (                    -- single row; written by the replay job's definers only
  singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
  status text NOT NULL CHECK (status IN ('idle', 'replaying', 'rebuilding_work', 'checking', 'complete', 'failed')),
  recovery_epoch bigint NOT NULL, started_at_ms bigint, events_total bigint, events_applied bigint,
  last_progress_at_ms bigint, failure_reason text);
CREATE TABLE finance.replay_check_record (            -- the independent check of 17.6; written by record_replay_check only
  check_id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  recovery_epoch bigint NOT NULL, result text NOT NULL CHECK (result IN ('pass', 'fail')),
  stream_heads_digest bytea NOT NULL CHECK (octet_length(stream_heads_digest) = 32),
  journal_sequence bigint NOT NULL, checked_at_ms bigint NOT NULL, detail_code text);
CREATE UNLOGGED TABLE finance.flow_lock (               -- the lock set of this transaction, written by lock_flow only
  txid bigint NOT NULL, table_name text NOT NULL, pk bytea NOT NULL, PRIMARY KEY (txid, table_name, pk));
```

`finance.event_marker` and `finance.flow_lock` are definer-written, belong to no flow's rank (a
transaction only ever inserts its own rows, so they never wait), and are swept for transactions
that are no longer in progress.

The constraints that make an unbalanced, forged or mis-classed journal impossible (the first is a
privilege, the rest are triggers and checks that stay in force even if the posting function had a
defect):

1. **Posting only through the definer.** No runtime or service role has `INSERT`, `UPDATE`, `DELETE`
   or `TRUNCATE` on any finance table. `df_finance` has `SELECT` and `EXECUTE` on the definer
   functions of 17.6, and the lines reach the journal only through `finance.commit_event`, whose
   `entry_kind_rule` allowlist decides which accounts and sides each entry kind may touch. Without
   this, any balanced entry would commit (for example Dr expense / Cr `purchased_available`, which
   mints refundable credit with no payment).
2. **Balanced per book and currency, checked at commit.** One check function, fired by two
   `CONSTRAINT TRIGGER ... DEFERRABLE INITIALLY DEFERRED FOR EACH ROW`, one `AFTER INSERT ON
   journal_entry` and one `AFTER INSERT ON journal_line`. It counts the entry's lines (must equal
   `line_count`, so a line added to an earlier entry in a later transaction fails that transaction)
   and requires `sum(amount_micro_dollars) = 0` for every (book, currency) group of the entry. An
   entry without lines, with a line count that disagrees with its lines, or with an unbalanced group
   cannot commit. The books are separate groups, so a line cannot cross from Promo to Exposure.
   `post_entry` runs `SET CONSTRAINTS` on the named finance constraint triggers to `DEFERRED` as its first step (never `ALL`, which would change other owners' constraints), so a caller that has
   issued `SET CONSTRAINTS ALL IMMEDIATE` cannot make the check fire early and break a multi-row
   insert; a bypass attempt at worst fails the caller.
3. **Append-only.** `BEFORE UPDATE OR DELETE` triggers on `journal_entry`, `journal_line`,
   `ledger_account` raise; a statement-level `BEFORE TRUNCATE` trigger raises on every journal and
   subledger table that must never be truncated; `UPDATE, DELETE, TRUNCATE` are revoked from every
   role; the tables are owned by `df_migrator`.
4. **Enforced balances never negative, and always present.** Creating an account key of an
   enforced code inserts its `account_balance` row in the same statement (an `AFTER INSERT` trigger on
   `account_key`), and a deferred check requires every enforced line's key to have a row. The
   balance rows are updated by `commit_event` after it has locked them in key order (19.1); the
   `CHECK (natural_micro_dollars >= 0)` refuses the update at once, so a reservation larger than an
   allocation fails inside the admission transaction. `account_balance` is writable only by the
   definer owner.
5. **Lot class and currency on every key.** A `BEFORE INSERT` trigger on `account_key` (keys are
   created only by definers, 17.6) requires, for codes 2110, 2120, 2130, 2140 and 2500, that the
   key's lot is a `purchased` lot, and for 2210, 2220 and 2230 a `promotional` lot; that the key's
   currency equals the lot's; that the payer on the key equals the lot's payer; and, for allocation
   keys, that the allocation's payer is the lot's payer. A promotional lot can therefore never sit
   under a purchased code, and a key cannot exist for a lot of another payer or currency.
   `lot_id` and `allocation_id` are real foreign keys (added in 0021).
6. **Key shape.** `key_shape_is_valid` requires exactly the dimensions the chart names for the code
   (a purchased-available key without a lot cannot exist).
7. **Approver is not the actor.** `CHECK (approver_id <> actor_id)` above, and `post_entry` requires
   an approver for the kinds and amounts that need one (17.6).
8. **Reversal.** `UNIQUE (reverses_entry_id)`, and a deferred trigger requires the reversal's lines to
   be the exact negation of the original's, line for line, and the original's kind to be reversible
   (16.17).
9. **Closed periods and the period of an entry** are the database's, not the caller's (17.8).
10. **Hash chain.** `prev_hash` must equal the stream head's `last_hash` and `stream_seq` the head's
    `last_seq + 1`, both read under the head's row lock in `post_entry`; a deferred trigger
    re-checks them (17.7).

Also in 0020: `finance.operation_receipt (namespace, principal, operation_id, fingerprint, outcome,
result_ref, created_at_ms)` with namespaces (`admission`, `allocation`, `withdrawal`,
`policy_update`, `quote_approval`, `refund`, `credit`, `manual_adjustment`); a receipt exists only
for an operation that changed state (11.2), and `finance.retired_operation_namespace` (B's 90-day
pattern).

### 17.2 `0021_finance_funding.sql`

| Table | Key columns and constraints |
|---|---|
| `payer` | `account_id` pk (B's account id, validated through B's account directory when the row is created; no cross-owner foreign key, 19.1), `standing` (`active`, `restricted`), `standing_reason`, `revision`, `created_at_ms`. The per-payer lock anchor (19.1) and the standing mirror B writes (19.4) |
| `funding_lot` | `lot_id` pk, `payer_account_id` FK payer, `class` (`purchased`, `promotional`), `promo_source` (`trial`, `goodwill`) null, `currency`, `original_micro_dollars > 0`, `state` (`active`, `frozen`, `charged_back`, `closed`, `forfeited`), the running totals `consumed_micro_dollars`, `refunded_micro_dollars`, `charged_back_micro_dollars` (the disputed amounts lost, informational and used for the state), `extinguished_micro_dollars` (liabilities removed by a lost dispute: the frozen and refund-payable parts, and recoveries), `rounded_micro_dollars`, `expired_micro_dollars`, `forfeited_micro_dollars` (all `>= 0`), `processor`, `payment_ref` null, `charge_ref` null, `purchase_order_ref` null, `tax_mode`, `tax_jurisdiction`, `tax_rate_numerator`, `tax_rate_denominator`, `tax_micro_dollars`, `billing_country` and `billing_region` (both nullable and **never journaled**, rebuilt from B after a restore, 22.1), `evidence_state` (`complete`, `pending_rebuild`), `tax_policy_version`, `created_at_ms`, `expires_at_ms` null, `revision`. Unique (`processor`, `payment_ref`); `CHECK ((class = 'purchased') = (payment_ref IS NOT NULL))`; `CHECK ((class = 'promotional') = (promo_source IS NOT NULL))`; `CHECK (state <> 'forfeited' OR class = 'promotional')`. The lot's own keys (2110/2140/2500 or 2210) are created in the statement that creates the lot |
| `processor_payment_state` | pk (`processor`, `payment_ref`), `lot_id`, `captured`, `refunded_total_micro_dollars`, `dispute_ref`, `dispute_status`, `disputed_micro_dollars`, `object_revision`, `applied_at_ms`. Monotonic by merge in the definer (7.3); a trigger refuses a lower refunded total or an earlier dispute status as a backstop. Locked at rank 2 (19.1) |
| `trial_grant` | `account_id` pk, `email_grant_key bytea(32)` unique and nullable only while `evidence_state = 'pending_rebuild' after a replay (never journaled, 22.1), `lot_id`, `granted_at_ms`. Kept past account deletion. The `trial_grant` entry kind requires this row as its anchor |
| `price_version` | `price_version_id` pk, `label` (the opaque code), `effective_from_ms`, `currency`, `platform_rate_micro_dollars_per_hour`, `markup_numerator`, `markup_denominator`, `tax_policy_version`, `presets jsonb` (validated), `proposed_pilot`, `status` (`published`, `retired`), `tariff_digest`, `published_at_ms`, `published_by`. Immutable except `status` (trigger) |
| `price_version_tariff` | pk (`price_version_id`, `provider`, `model`, `category`, `context_tier_max_tokens`), `rate_nano_dollars`, `per_units`, `as_of`, `source`. Append-only. Publishing also creates the version's provider-dimension account keys (5100, 5110, 5200 by loss class, 2300, 2310, 1200, 9100, 9190 per provider) |
| `session_funding` | `session_id` pk (O1's table session id; validated at open, no FK across owners), `tenant_id`, `campaign_id` (S1's id, validated at open; no FK), `price_version_id` FK, `currency`, `status` (`open`, `settling`, `closed`), `policy_revision`, `hourly_ceiling_micro_dollars`, `session_ceiling_micro_dollars` null, `per_request_threshold_micro_dollars`, `split_mode`, `contributions_visible`, `generation_policy_digest`, `current_epoch`, `pace_tokens_micro_dollars`, `pace_updated_at_ms`, `meter_priced_ms`, `meter_charged_micro_dollars`, `meter_forgone_micro_dollars`, `meter_funded_until_ms`, `opened_at_ms`, `settling_at_ms` null, `closed_at_ms` |
| `session_policy` | pk (`session_id`, `policy_revision`), the policy fields, `changed_by_account`, `changed_at_ms`. Append-only history |
| `allocation` | `allocation_id` pk, `session_id` FK, `payer_account_id` FK payer, `authorized_micro_dollars`, `drawn_micro_dollars`, `returned_micro_dollars`, `charged_micro_dollars` (the subledger totals of 17.3), `status` (`active`, `paused_departed`, `consent_incompatible`, `withdrawn`, `closing`, `closed`), `consent_revision`, `created_at_ms`, `ended_at_ms`. Unique (`session_id`, `payer_account_id`) |
| `allocation_consent` | pk (`allocation_id`, `consent_revision`), `price_version_id`, `approved_hourly_ceiling`, `approved_session_ceiling`, `approved_split_mode`, `approval_threshold_micro_dollars`, `generation_policy_digest`, `consent_terms_version`, `consented_at_ms`. Append-only |
| `allocation_lot` | pk (`allocation_id`, `lot_id`), `draw_rank`. Registers which lot keys an allocation holds; the allocation's keys for the lot are created when the row is |
| `funding_epoch` | pk (`session_id`, `epoch_seq`), `reason` (closed), `split_mode`, `created_at_ms` |
| `epoch_share` | pk (`session_id`, `epoch_seq`, `allocation_id`), `weight` (> 0) |
| `funding_feed_position` | **F's own cursor** (D31), per campaign: `campaign_id` pk, `last_event_seq`, `feed_epoch bytea(16)` (S1's 128-bit random value, not a counter), `updated_at_ms`. It is authoritative for the `funding` consumer. **The row is created in the same transaction as F's first payer grant (allocation) for the campaign, from the `event_seq` and `feed_epoch` of the `member_snapshot` that transaction read** (S1 0h; 8.2). It is advanced in the same transaction as the effect it acknowledges, and the effect rows (`funding_effect`) are unique on (`campaign_id`, `event_seq`), so a crash between an effect and the acknowledgement cannot apply an event twice and a replay is a no-op. S1 keeps no cursor table. There is no lag column and no lag gate (11.2). The effect transaction locks only F rows (the position row at rank 7, 19.1) |
| `funding_effect` | pk (`campaign_id`, `event_seq`), `effect` (closed: `paused_departed`, `released_unreserved`, `none`), `allocation_id` null, `applied_at_ms` |

**Catch-up and the first grant (S1 F-3, round 0h).**
- *First grant.* F's first `allocate` for a campaign reads the fresh `member_snapshot` that
  authorizes the payer and, **in the same transaction**, inserts the campaign's position row at that
  snapshot's `event_seq` and `feed_epoch` (`INSERT ... ON CONFLICT DO NOTHING`; if the row already
  exists it is left alone). Any event with a higher `event_seq`, for example a `left` that commits
  after the snapshot but before the grant commits, is therefore applied by the consumer, once.
- *Sweep.* At startup and every `feed_reconcile_seconds = 30`, F sweeps S1's
  `campaign_heads_after(after_campaign_id, limit)` (keyset on the primary key). A campaign F has
  no row for gets a position at its head, taken in the same statement as a fresh `member_snapshot`,
  **inserted with `ON CONFLICT DO NOTHING`**, so the sweep can never overwrite a position that a
  concurrent first grant just wrote (F has no allocation in a campaign it has no row for, so
  nothing needs replaying). A known campaign replays its events from its stored position, each
  through `funding_effect`.
- *Regression.* An `event_seq` below the stored position, or a changed `feed_epoch`, makes F
  re-derive the **status of each allocation it holds in that campaign** (`paused_departed`,
  `withdrawn`, active) from the fresh snapshot instead of trusting its stored position, and alert.
- S1's `campaign_changes_since` is a discovery hint only.
- Tests: `f_down_across_three_events_applies_each_once`; the interleaving
  `a_sweep_never_overwrites_a_position_written_by_a_concurrent_first_grant` (the grant takes its
  snapshot, a `left` event commits, the sweep runs, the grant commits: the `left` is applied once,
  not skipped); `f_creates_its_position_row_in_the_transaction_of_its_first_grant` (26.1 POT-04).

### 17.3 `0022_finance_admission.sql`

| Table | Key columns and constraints |
|---|---|
| `quote` | `quote_id` pk, `session_id`, `request_digest`, `price_version_id`, `work_class`, `retail_estimate`, `retail_maximum`, `supplier_maximum`, `created_at_ms`, `expires_at_ms`, `state` (`open`, `approved`, `declined`, `expired`, `used`), `used_by_reservation` null. Unique (`session_id`, `request_digest`, `price_version_id`) where open. Locked at rank 4 (19.1) |
| `quote_approval` | pk (`quote_id`, `allocation_id`), `decision` (`approved`, `declined`), `consent_terms_version`, `at_ms`. Append-only |
| `reservation` | `reservation_id` pk, `session_id` null (null for company), `payer_kind` (`table`, `company`), `company_class` null, `kind` (`provider`, `time`), `operation_id`, `quote_id` null **unique where not null** (a quote funds one reservation), `price_version_id`, `retail_maximum_micro_dollars`, `admitted_at_ms`, `state` (`held`, `charged`, `released`), `charged_micro_dollars`, `released_micro_dollars`. `CHECK (charged + released <= retail_maximum)`; `CHECK (state = 'held' OR charged + released = retail_maximum)` (company reservations have a zero retail maximum) |
| `company_exposure_counter` | pk (`scope`, `period`), `used_micro_dollars`. Exists only for a configured company limit (11.2 step 4); with every limit unlimited (the default) no row is read or locked |
| `reservation_share` | pk (`reservation_id`, `allocation_id`, `lot_id`), `class`, `held_micro_dollars > 0`, `charged_micro_dollars >= 0`, `released_micro_dollars >= 0`, `reversed_micro_dollars >= 0`, `CHECK (charged + released <= held)`, `CHECK (reversed <= charged)` |
| `provider_attempt` | `attempt_id` pk (D22 shared id), `reservation_id` unique FK, `effect_id`, `campaign_id` null (null for the account principal), `account_id` null, `tenant_id`, `provider`, `model`, `route`, `media_asset_id` null, `supplier_maximum_micro_dollars`, `supplier_exposure_micro_dollars` (the worst case held in the Exposure book while `ExposureOpen`), `dispatch_state` (`reserved`, `dispatch_prepared`, `sent`, `result_known`, `outcome_unknown`, `verified_unsent`, `cancelled_unsent`, `failed_unknown`), `customer_state`, `supplier_state`, `dispatcher_instance_id` null, `supplier_outcome` (`billed`, `not_billed`, `unknown`) null, `usage_source` null, `supplier_cost_micro_dollars` null, `usability` null, `usability_class` null (O2's label), `provider_request_ref` null (operator only), `prepared_at_ms`, `sent_at_ms`, `result_at_ms`, `customer_released_at_ms`, `settled_at_ms`, `fence_recovery_epoch`, `fence_claim_seq`, `fence_placement_id` (recorded at admission, prepare and send). A state-transition trigger allows only the arrows of 11.3 |
| `usage_event` | `usage_event_id` pk, `attempt_id` unique null, `meter_segment` unique null (`session_id`, `segment_seq`), `session_id`, `category` (customer category, closed), `quantity_text` (customer units, e.g. `42m 18s`, produced by the closed formatter of 18.9), `units jsonb` (exact provider units; operator only), `retail_micro_dollars`, `supplier_micro_dollars`, `supplier_purchased_micro_dollars`, `supplier_promotional_micro_dollars`, `reversed_micro_dollars` (`CHECK (reversed <= retail)`), `reclassified_micro_dollars`, `price_version_id`, `entry_id`, `settled_at_ms`. `CHECK ((attempt_id IS NULL) <> (meter_segment IS NULL))`: every charge maps to one attempt (with its quote or policy) or one meter segment (plan 15.4). Locked `FOR UPDATE` by the credit flow (12.5) |
| `usage_reversal` | pk (`usage_event_id`, `operation_id`), `micro_dollars`, `reason`, `entry_id`. Append-only; the lock and the two CHECKs above are the guard, not a sum trigger |
| `meter_segment_priced` | pk (`session_id`, `segment_seq`), `priced_ms`, `credited_ms`, `charge_micro_dollars`, `forgone_micro_dollars`, `entry_id` |
| `forgone_revenue` | pk (`reason`, `ref_id`), `reason` (`meter_unfunded`, `refunded_externally`), `session_id`, `micro_dollars`. Report only |
| `company_loss` | pk (`attempt_id`, `loss_class`), `micro_dollars`, `entry_id` |
| `settlement_fact` | `attempt_id` pk, `source` (`runtime`, `o2_outbox`, `deadline`), `outcome`, `reason` (O2's label), `outbox_seq` null, `recorded_at_ms`. The idempotency anchor of the O2 consumer (12.6) |
| `settlement_feed_position` | consumer of O2's `media.settlement_outbox`: the last consumed outbox id, a hint (12.6) |
| `attempt_outcome_event` | `event_seq` bigserial, `attempt_id` unique, `media_asset_id`, `outcome` (`failed_unknown`, `cancelled_unsent`), `reason` (`unknown_outcome_failed`, `not_dispatched`), `recorded_at_ms`. O2's consumer reads it (12.6) |
| `work_item` | `work_id` pk, `kind` (12.3), `ref_id`, `due_at_ms`, `claimed_by`, `lease_until_ms`, `generation`. Partial index on due items |

**Commit-time triggers tying the subledgers to the keys** (deferred constraint triggers, so they see
the whole transaction; they are the third guard after the definer's own checks and the daily job):

- *Reservations.* For every (allocation, lot) touched by a change to `reservation_share`: the sum of
  `held - charged - released` over its shares equals the natural balance of that allocation's
  `*_reserved` key of the right class (2130 or 2230). An index on (`allocation_id`, `lot_id`)
  restricted to open shares keeps the sum bounded by the open reservations.
- *Allocations.* For every allocation touched: `drawn - returned - charged = allocated + reserved`
  over all its keys (both classes). `allocation_lot` registers the lots; the totals are on the
  allocation row.
- *Lot state versus balances.* A lot becomes `closed` only when every one of its keys is zero, it
  has no open share and no refund lot not final; `frozen` only when the dispute row is open;
  `charged_back` only with `purchased_frozen` zero; no key of a `closed` or `forfeited` lot may
  hold a balance.
- *Conservation.* On every change to a lot row or one of its keys, **one SQL statement** (a single
  query that joins the lot row to the sum of its keys' balances, so the check reads one snapshot and
  cannot see a half-updated lot):
  `original = available + allocated + reserved + frozen + refund_payable + consumed + refunded +
  extinguished + rounded` (purchased), and `granted = available + allocated + reserved + consumed +
  expired + forfeited` (promotional), with the totals on the lot row. `extinguished` is what a lost
  dispute removes from the liabilities (14.2: frozen and refund-payable parts, plus later
  recoveries); the consumed and reserved parts that the loss covers stay in `consumed` and
  `reserved`. Every flow that changes a lot total takes the lot `FOR UPDATE` (19.1), so the totals
  are never stale and a concurrent settlement on the same lot waits instead of deadlocking.
- *Allocation closing.* An allocation may be `closed` only with `allocated = 0` and `reserved = 0`.

### 17.4 `0023_finance_refunds_disputes.sql`

| Table | Key columns and constraints |
|---|---|
| `refund` | `refund_id` pk, `payer_account_id`, `operation_id` unique per payer, `requested_micro_dollars`, `state` (`pending`, `paid`, `partly_paid`, `review`, `returned`), `requested_at_ms`, `released_allocations jsonb` |
| `refund_lot` | `refund_lot_id` pk (also the processor idempotency key), `refund_id`, `lot_id`, `principal_micro_dollars`, `tax_micro_dollars`, `processor_refund_ref` null, `payout_method` (`processor`, `manual`), `state` (`pending`, `unknown`, `paid`, `failed_review`, `returned_to_credit`, `manual_pending`, `blocked_by_dispute`, `cancelled_dispute_lost`), `object_revision`, `updated_at_ms`. Unique (`refund_id`, `lot_id`) |
| `refund_outbox` | `refund_lot_id` pk, `claimed_by`, `lease_until_ms`, `attempts`, `next_attempt_at_ms`. B's processor worker claims with `FOR UPDATE SKIP LOCKED` and a lease plus attempt generation (plan 6.6). `blocked_by_dispute` and `manual` lots are not offered |
| `dispute` | pk (`processor`, `dispute_ref`), `lot_id`, `disputed_micro_dollars`, `status` (`open`, `won`, `lost`), `fee_micro_dollars`, `opened_at_ms`, `closed_at_ms`. Monotonic status |
| `manual_adjustment` | `adjustment_id` pk, `entry_id`, `actor_id`, `approver_id` null, `case_ref`, `reason_text` (redacted after the target account's deletion, B's rule), `created_at_ms`; `CHECK (approver_id IS NULL OR approver_id <> actor_id)` |
| `tax_policy` | pk `tax_policy_version`, `mode`, `jurisdictions jsonb`, `effective_from_ms`. Append-only |

### 17.5 `0024_finance_reconciliation.sql`

| Table | Key columns and constraints |
|---|---|
| `reconciliation_run` | `run_id` pk, `kind` (`journal_integrity`, `journal_compare`, `integrity_controls`, `processor`, `payouts`, `bank`, `supplier_usage`, `reservations`, `meter`, `subledger`, `outcome_feeds`, `campaign_restore`, `full`), `scope` (period or campaign), `as_of_ms`, `watermark_journal_sequence`, `stream_heads_digest bytea(32)` (SHA-256 over the sorted (stream, last_seq, last_hash) read in one statement), `started_at_ms`, `finished_at_ms`, `outcome` (`clean`, `differences`, `failed`) |
| `reconciliation_difference` | `difference_id` pk, `run_id`, `kind` (closed), `reference`, `expected_micro_dollars`, `observed_micro_dollars`, `state` (`open`, `within_tolerance`, `explained`, `corrected`), `explanation_code`, `corrected_by_entry` null. Never resolved by a plug entry: `corrected` needs a corrective or reversing entry with a case (plan 7.3) |
| `reconciliation_signoff` | pk (`run_id`), `reviewer_id`, `signed_at_ms`, `note_code`. The reviewer must differ from whoever closes the period (17.8) |
| `supplier_invoice` | `invoice_id` pk, `provider`, `period`, `imported_at_ms`, `digest`, `total_micro_dollars` |
| `supplier_invoice_line` | pk (`invoice_id`, `line_no`), provider, model, category, quantity, `micro_dollars`, `attempt_ref` null |
| `exposure_match` | pk `attempt_id`, `invoice_id`, `matched_micro_dollars`, `entry_id` |
| `bank_statement_line` | `line_id` pk, `statement_date`, `bank_transaction_ref` unique, `amount_micro_dollars`, `imported_at_ms`, `matched_entry_id` null. Imported by an operator (23.2) |
| `period_balance` | pk (`period_id`, `key_id`), `opening`, `debits`, `credits`, `closing`. Written once by `close_period` (17.8) |
| `export_manifest` | pk (`period_id`, `kind`, `format`, `generation`), `digest bytea(32)`, `watermark_journal_sequence`, `stream_heads_digest`, `as_of_ms`, `generated_at_ms`, `provisional` |
| `restore_replay` | pk (`recovery_epoch`, `journal_sequence`), `event_id`, `applied_at_ms` (OPS-02 idempotency) |
| `reconstruction` | pk `reconstruction_id`, `tenant_id`, `campaign_id`, `outcome` (`reconstructed`, `failed`), `finished_at_ms` (22.2) |

### 17.6 The posting boundary: `lock_flow`, `commit_event`, and the entry-kind allowlist

**Why a boundary.** The round-0 design let the finance role insert journal lines, so any balanced
entry committed. The rule now is: Rust plans, PostgreSQL enforces, and the only path from the
service to a finance table is a definer function. `df_finance` holds `SELECT` (reads, views,
exports) and `EXECUTE`; it holds no `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`, no ownership, no
`SET` on `session_replication_role`, and it cannot `DISABLE TRIGGER` (that needs ownership). Row
locks (`SELECT ... FOR UPDATE`) also need an `UPDATE` privilege, so they too go through definers.

**The two calls of every flow** (both `SECURITY DEFINER`, owner `df_finance_definer`, `SET
search_path = pg_catalog, finance, df_runtime, pg_temp`):

1. `finance.lock_flow(flow, args) -> jsonb`. Takes the locks of the flow in the rank order of 19.1
   (after `enter_service()`) and returns the rows it read under them: the snapshot the pure
   decision runs on. A flow is one of `admission`, `settlement`, `release`, `meter`, `allocate`,
   `withdraw`, `refund`, `dispute`, `credit`, `processor_state`, `close_lots`, and so on. For flows
   that begin from an attempt it resolves the attempt to its session and payers with an
   **unlocked read**, locks session, allocations and lots in order, then locks the attempt-level
   rows (rank 7) and re-checks the state; a changed state returns `Stale` and the caller does
   nothing.
2. `finance.commit_event(event jsonb) -> jsonb`. The event is the serialized `FinancePlan`:
   `{ event_id, event_kind, entries: [...], changes: [...] }`. In order, in one call, it:
   - checks `event_kind` against `event_kind_rule` (which entry kinds and which `RowChange` kinds
     this event may carry);
   - takes the accounting-period advisory lock (shared, 17.8) and derives `period_id` from the
     database clock;
   - locks every ledger-stream head the entries name, in (kind, id) order, then every
     `account_balance` row they touch, in key-id order (ranks 8 and 9) **before any other
     step**, so no later O1 or media lock can sit under a balance lock out of order;
   - for each entry, checks every line against `entry_kind_rule`, the anchor, the external
     reference, the actor and approver rules, and the key resolution (a line may name only a key
     that already exists: keys are created by the definers that create their lot, allocation or
     price version, never by posting); inserts the entry (hash chain, 17.7) and its lines in a
     single multi-row statement; applies the balance deltas;
   - applies the row changes (`RowChange`) through `apply_change`, each a typed after-image: insert
     for append-only tables, compare-and-set update on `expected_revision` (below)
     for mutable ones; nothing else writes a subledger table;
   - inserts the same event, with its entry hashes, into the recovery journal's pending table
     through O2's `journal.submit_pending(kind, stable_id, payload, metadata)` (request to O2:
     `EXECUTE` to `df_finance_definer`; kind `finance_event`; `metadata` carries the event id, the
     stream kind and id of every entry, and the campaign id, so the journal reader can select by
     stream and campaign, 22.2; the payload is the **allowlisted fields of 22.1** and no personal
     data), in the same transaction, and records the `event_marker` row for the transaction;
   - checks the per-kind equations (below) and returns the new ids.
   The helpers (`post_entry`, `apply_change`) are not granted to any role. Every other definer,
   including the dispatch definers of 11.4, builds its small event and calls `commit_event`.

**Compare-and-set and the lock set.** Every mutable-row `RowChange` carries the `expected_revision`
the plan was computed from, and `apply_change` runs `UPDATE ... WHERE <pk> AND revision =
$expected_revision`; zero rows raises `StaleChange` and the event rolls back. "Forward-only"
alone was not enough: two after-images computed from revision n could both advance and one would
overwrite the other. In addition, `lock_flow` records each row it locks in the definer-written
`finance.flow_lock (txid, table, pk)` (17.1), and `apply_change` refuses a change to a row that is
not in the transaction's lock set (inserts of new rows are exempt), so a plan cannot touch a row the
flow did not lock, in the right order, in the first place. Mutant: drop `expected_revision` (the
lost-update test of 26.2 must fail); mutant: skip the lock-set check.

**Per-kind equations, enforced inside `commit_event`.** The allowlist checks only (code, side). A
plan defect could still move value between two payers' keys, or post revenue unrelated to the usage
event. So after the lines and the row changes are staged and before the commit, `commit_event`
checks, for each entry, the equations of its kind against the row changes of the same event (the
rows it has just written or updated), and against the lines' dimensions:

| Id | Kind | Equation | Mutant (the test of 26.2 must fail) |
|---|---|---|---|
| E1 | `settlement`, `meter_charge` | the sum of the 4110 (or 4100) credit lines equals the inserted `usage_event.retail_micro_dollars`; the 5100 + 5110 debit lines equal `usage_event.supplier_micro_dollars` and each equals its `supplier_purchased/promotional` field | revenue line increased by one micro-dollar |
| E2 | `settlement`, `meter_charge`, `release`, `company_loss` | for each reserved key, the debit equals the change in that share's `charged + released`; the credits to revenue plus the return lines equal the sum of those changes | debit one share's key for another's amount |
| E3 | `settlement`, `meter_charge`, `release`, `company_loss`, `customer_credit`, `allocation`, `allocation_release`, `reservation`, `meter_hold`, `refund_requested`, `external_refund` | **membership**: every line whose key carries a payer, lot or allocation must belong to the anchoring row set (the reservation's shares, the allocation, the refund lot, the dispute's lot, the usage event's shares); a line over any other payer's or lot's key is refused | post to a key of another payer |
| E4 | `settlement`, `meter_charge`, `release` | each return line equals the `return_target` key the SQL recomputes from the lot state and allocation status read under the lock (14.4) | return to `*_allocated` for a Frozen lot |
| E5 | `customer_credit` | the credit lines equal the change in `usage_event.reversed` and in the shares' `reversed`; the reclass lines equal the change in `reclassified` | credit one micro-dollar more |
| E6 | `reservation`, `meter_hold` | the reserved credits equal `reservation.retail_maximum` and the sum of the shares' `held`; the exposure lines equal `provider_attempt.supplier_exposure` | exposure line differs from the attempt |
| E7 | `allocation`, `allocation_release` | the amounts equal the change in `allocation.drawn` or `returned` | release more than recorded |
| E8 | `purchase`, `external_refund` | the credit to `purchased_available` equals the lot's `original` (net of tax); an external refund equals the change in the payment state's refunded total, drained in the order of 14.4 | purchase amount differs from the payment state |
| E9 | `refund_requested`, `refund_paid`, `manual_refund_payout`, `refund_returned_to_credit` | the amounts equal the `refund_lot` principal (and tax) | pay a different amount |
| E10 | `dispute_freeze`, `dispute_withheld`, `dispute_won`, `dispute_lost` | the withheld and fee amounts equal the `dispute` row; the frozen amounts equal the lot's available and unreserved allocated balances read under the lock; `dispute_lost` follows the order and non-negativity of 14.2 (`x1 + x2 + x3 = D`, `x3 >= 0`, release `= f - x1`; only `pending`, `failed_review`, `manual_pending` and `blocked_by_dispute` refund lots enter `x2`, never `unknown`, `paid`, `returned_to_credit` or `cancelled_dispute_lost` ones) | negative loss; skip the release line; count an `unknown` refund lot in `x2`; leave a `manual_pending` or `blocked_by_dispute` lot payable |

An equation failure raises and rolls the event back. The pure constructors satisfy the same
equations; a test compares the two over the property sequences (26.3).

**Journaled event kinds: every writer.** The only write path to a finance table is `commit_event`,
so every state-writing definer is a **journaled event kind** by construction. The kinds are rows of
`finance.event_kind` (17.1), all journaled (`CHECK (journaled)`):
- flow events that carry entries: the entry kinds of the table below;
- record events with no entry: `price_version_published` (the version row, its tariff rows and its
  provider-dimension keys; in-flight sessions pinned to a version can be priced after a restore only
  because this is journaled), `payer_registered`, `payer_standing_set`, `session_funding_opened`,
  `session_policy_changed`, `allocation_consented`, `quote_created`, `quote_decided`,
  `attempt_prepared`, `attempt_prepare_cancelled`, `attempt_sent`, `attempt_unsent`,
  `transport_outcome`, `usage_recorded`, `settlement_fact_recorded`, `outcome_event_recorded`,
  `reconciliation_run_recorded`, `reconciliation_signoff`, `supplier_invoice_imported`,
  `bank_statement_imported`, `period_precreated`, `period_closed`, `time_zone_changed`,
  `trial_key_rebuilt`, `lot_evidence_rebuilt`.
The ephemeral tables that are not journaled are named in a closed list: `work_item` (re-derived
from state), `replay_state`, `restore_replay` and `replay_check_record` (restore bookkeeping), `flow_lock`, `event_marker`,
`settlement_feed_position` and `funding_feed_position` (hints rebuilt by the sweeps). **Write guard:**
a deferred row trigger on every table outside that list
requires an `event_marker` row for the current `txid`, so a definer that wrote without going
through `commit_event` fails at commit. The catalog-walk test `every_write_path_to_a_finance_table_is_a_journaled_event_kind`
lists the definers that can write (by `pg_proc` source and dependencies) and requires the set to
be `{commit_event, replay_event, apply_change, post_entry}` plus `lock_flow` (which writes only `flow_lock`); a
second part runs every flow and requires one journal pending row of the declared kind per
transaction. Mutant: add a definer that writes a table directly.

**Unjournaled changes.** `work_item`, `replay_state`, `restore_replay` and `replay_check_record` rows (and `flow_lock`,
`event_marker`) are written only by `apply_change`, as changes of the **unjournaled kinds**
`work_item_upsert`, `work_item_complete`, `replay_state_set`, `restore_replay_record` and `replay_check_record`. The
work-item changes ride inside whatever event creates the condition (for example `mark_attempt_sent`
writes the `sent_reconcile`, `customer_hold` and `unknown_resolution` items in its own event). The
replay job's definers, `replay_begin`, `replay_complete` and `rebuild_work_items` (17.9), and `record_replay_check`, are not
writers of their own: each builds its unjournaled changes and calls `apply_change`, so
`replay_state` and `restore_replay` sit behind the same compare-and-set and lock-set rules as
everything else, and the write guard exempts only the closed list of unjournaled tables (below)
because `apply_change` is the one function allowed to write them. They are re-derived or
re-created after a restore (22.2 step A.5), so they are not in the journal payload. The
catalog-walk set of writers is therefore exactly `{commit_event, replay_event, apply_change,
post_entry, lock_flow}`; a definer that writes `replay_state`, `restore_replay` or `work_item`
directly is outside the set and fails the walk (mutant: `replay_complete` writes `replay_state`
itself).

**Payload schema versions.** Every event carries `payload_schema` (an integer, stamped by
`commit_event` from the code that built it) beside its `event_kind`, and so does every sealed
journal entry. The journal outlives the code, so a restore can replay after-images written by an
older release under a newer `apply_change`. `apply_change` therefore **upgrades**: it holds one
upgrade function per (change kind, old version) that maps an old after-image to the current one
(adding a column with its documented default, renaming, splitting), upgrades are pure and total, and
an event with a `payload_schema` newer than the running code, or older than the oldest supported,
is refused and stops the replay with an alert (never skipped). A migration that changes a table
that appears in a payload must add the matching upgrade function in the same change (a gate test
lists the table columns against the current schema's after-image fields).

**Entry hash versions.** The payload schema and the entry hash are versioned separately, because
the hash is over the canonical bytes of the *entry* and a later release can add an entry field
(round 0c added `corrects_period_id`); a hash that silently covered a new field could not be
recomputed for an old entry. So every entry carries **`entry_hash_version`** (a small integer,
stamped by `post_entry` and sealed with the entry in the journal, 22.1), and **the canonical
encoding of each version is frozen**: version 1 hashes exactly the fields (the entry and its correlation columns, the lines) and the order listed in
17.7 item 2, and a new version is a new function that is added beside the old ones, never an edit
of one. `post_entry` always writes the current version. `replay_event`, `journal_compare` and the
integrity job recompute an entry's hash with the **version stored on that entry**, never the current
one, and verify the chain link (`prev_hash` equals the previous entry's stored `entry_hash`) without
re-encoding anything. The functions of all versions ship in every release (a manifest lists them,
and the controls job of 17.7 item 4 checks their body hashes). Test: a fixture journal containing
version-1 entries is replayed and compared by a build whose current version is 2, and every hash
and chain link verifies (26.2); mutant: recompute with the current version.

**`replay_event`: the separate apply path used only at restore.** `commit_event` is for live
postings: it reads the database clock for `recorded_at_ms` and `period_id`, recomputes the hash
and the stream position, applies the approver and period checks, and inserts a new pending journal
row. None of that is right for a replay: the time and period must be the original ones, the
journal entry already exists, and the closed-period and approver rules were satisfied when the event
was first posted. So a replay uses its own definer, `finance.replay_event(event jsonb)`:
- *Who.* Granted only to **`df_finance_replay`, a dedicated LOGIN role** with its own credential in
  S2's secrets layout, `CONNECTION LIMIT 2`, used by the replay job's own connection and by nothing
  else. It is **not assumed by `SET ROLE` from `df_finance`**, and `df_finance` is not a member of
  it, so the live service connection cannot reach `replay_event` at all; the state gate below is a
  second control, not the only one. The replay job runs as its own task of the F service with that
  separate pool (19.3) and holds no live-posting privilege.
- *When.* Refused unless O1's service state is **`restoring`** (below) and F's own
  `finance.replay_state` says a replay is in progress. It reads the state through O1's lock-free
  `df_runtime.service_state()`. In any other state, including `serving`, it raises `NotReplaying`, so
  a live caller can never use it to write history.
- *What it takes from the payload.* `recorded_at_ms`, `effective_at_ms`, `period_id`, `stream_seq`,
  `prev_hash`, `entry_hash` and `entry_hash_version` come **from the payload**; it never reads the
  clock. A closed period does not refuse a replayed entry whose period it names, because the period
  row itself is replayed from the `period_closed` event first (and its snapshot and digest are
  re-verified).
- *What it verifies.* It recomputes the entry hash from the payload's canonical bytes **with the
  stored `entry_hash_version`** and requires it to equal the stored `entry_hash` (a hash-match check:
  a corrupted or tampered payload fails), requires `prev_hash` to equal the stream head's
  `last_hash` and `stream_seq` to be head + 1, runs the per-kind equations of the table above, the
  allowlist and the key checks, and applies the row changes through `apply_change` (after the schema
  upgrade).
- *What it writes.* The entry, its lines and the balance deltas (same constraint triggers), the row
  changes, the `event_marker`, and `restore_replay` (through `apply_change`). It does **not** call
  `submit_pending`: the journal entry already exists, so replay does not duplicate it.
- *The independent check, and how the database enforces that it happened.* `replay_event` applies
  events on the strength of the journal, so the control on what it wrote cannot be itself. It is the
  **post-replay check of 22.3**, which runs as a different role (`df_finance`, read only) and on a
  different code path, and **the database refuses to finish the replay without its result**:
  - `finance.record_replay_check(result, stream_heads_digest, journal_sequence)` is a definer
    granted to `df_finance` and **not to the replay role**. It writes a row of the unjournaled kind
    `replay_check_record` (through `apply_change`, like the other restore bookkeeping) carrying the
    result (`pass` or `fail`), the digest of the live stream heads that the check compared (one
    statement over the sorted (stream, last_seq, last_hash), as in 17.5) and the confirmed journal
    sequence it replayed up to. It is allowed only while the service is `restoring`.
  - `replay_complete` refuses (`CheckMissing` or `CheckStale`) unless a **`pass`** row exists whose
    `stream_heads_digest` equals the digest of the live stream heads **at the moment of
    completion** and whose journal sequence is at least the replay's high-water mark. A job that
    applied the events cannot set `complete` or call `restore_complete` without a passing check of
    exactly the state it left, and a change made after the check (any event applied afterwards)
    changes the digest and invalidates it. A failed check leaves the service `restoring`.
  - *Where the rebuild runs.* The check is an **in-memory model in Rust**: `df_finance` reads the
    confirmed journal through O2's reader and the live rows with `SELECT`, applies the typed
    after-images (the same `RowChange` values, through the pure `df-commerce` model, not through SQL)
    to an empty model, and compares the result row for row with the live tables and the stream
    heads. It needs no scratch schema and no DDL privilege, which a read-only role could not have.
    The model is the property-test model of 26.3, so the comparison and the tests share one
    implementation of the rules.
  - It is backed by the controls job of 17.7 and the next daily processor reconciliation.
- *Mutants and tests.* Replayed hashes equal the originals (`replayed_entry_hashes_equal_the_original_hashes`); a payload
  with one changed amount fails the hash check; `replay_event` raises while the service is
  `serving`; a live `commit_event` is refused during a replay; the replay never reads the clock
  (the test runs with the database clock moved a year forward and the periods and times are the
  originals); `df_finance` cannot execute `replay_event` or `SET ROLE` to the replay role; `replay_complete` refuses with no check row, a `fail` row, or a `pass` row whose digest differs from the live digest. Mutants: skip the check-row test in `replay_complete`; compare against a stale digest;
  skip the hash comparison; skip the state check; read the clock; grant `df_finance` membership of
  the replay role.

**The `restoring` serving state (a handshake with O1).** O1's committed 6.3 has the operator's
restore step raise the recovery epoch and set `serving = 'serving'` in one step, so no window exists
in which service is stopped while F replays. Two things must hold instead (ruling, round 0e,
option (a); request to O1): the restore step sets `df_meta.service_state.serving = 'restoring'`
(a new value of the enum beside `serving` and `halted_regression`); and **runtime claims,
`place`, `renew`, `fence_current` and F's admission all refuse while it is `restoring`** (the
existing `serving = 'serving'` predicates in O1's SQL already do for the directory and the fence; F's
admission answers `PaidWorkFrozen`). F's `replay_complete` is the only way back: after the replay
check passes it calls O1's definer `df_runtime.restore_complete(recovery_epoch)` (EXECUTE to
`df_finance_replay` only), in the same transaction that sets `replay_state` to `complete`
(finance rows first, then O1's), which moves `restoring` to `serving`. If the replay fails or
stalls, the state stays `restoring`: no runtime serves and nothing is dispatched. Without the
handshake the "not serving" gate could never be satisfied, so this is not optional.

**The allowlist.** `finance.entry_kind (kind, stream_rule, anchor, external_ref_kind, actors,
reversible, needs_approver_above)` and `finance.entry_kind_rule (kind, code, side)`, seeded by
0020 from `df_commerce::entry_rules` (a test compares the seed with the pure table, 26.2). A line
is accepted only when its (code, side) is in the kind's rows. The books balance independently, so
the kinds below also fix which books an entry touches. D = debit side, C = credit side;
"exposure" means 9100 D / 9190 C on open and the reverse on close.

| Kind | Stream | Allowed lines (code side) | Anchor and external reference | Actors |
|---|---|---|---|---|
| `trial_grant` | payer | 3910 D, 2210 C | `trial_grant` row | system |
| `goodwill_grant` | payer | 3910 D, 2210 C | `manual_adjustment` row with case | operator |
| `purchase` | payer | 1100 D; 2110 C, 2400 C | `processor_payment_state` row; ext `processor_payment` | system |
| `processing_fee` | platform | 5400 D; 1100 C | ext `processor_balance_txn` | system |
| `payout` | platform | 1000 D; 1100 C | ext `processor_payout` or `bank_transaction` | system |
| `allocation` | payer | 2110 D, 2120 C; 2210 D, 2220 C | allocation row | system |
| `allocation_release` | payer | 2120 D, 2110 C; 2220 D, 2210 C | allocation row | system |
| `reservation`, `meter_hold` | session (campaign for a company admission, account for a screen) | 2120 D, 2130 C; 2220 D, 2230 C; exposure open (not in `meter_hold`) | reservation row | system |
| `release` | session or campaign | 2130 D; 2120 C, 2110 C, 2140 C, 5500 C (recovery on a charged-back lot); 2230 D; 2220 C, 2210 C; exposure close | reservation row | system |
| `settlement`, `meter_charge` | session | 2130 D, 2230 D; 4110 C or 4100 C, 2400 C, 2120 C, 2110 C, 2140 C, 5500 C, 2220 C, 2210 C, 3910 C; 5300 D, 4200 C (gross mode only); 5100 D, 5110 D; 2300 C or 1200 C; exposure close | attempt or segment; usage event | system |
| `company_loss` | session or campaign | as `release`; 5200 D; 2300 C, 1200 C | attempt | system |
| `customer_credit` | session | 4110 D, 4100 D; 2110 C, 2140 C; 5200 D; 5100 C, 5110 C; 3910 D, 2210 C; 5300 C, 4200 D (gross only) | `usage_event` row | operator or system |
| `refund_requested` | payer | 2110 D; 2500 C | `refund_lot` row | system |
| `refund_paid` | payer | 2500 D, 2400 D; 1100 C | `refund_lot` row; ext `processor_refund` | system |
| `manual_refund_payout` | payer | 2500 D, 2400 D; 1000 C | `refund_lot` in `manual_pending`; ext `bank_transaction`; case | operator, approver above threshold |
| `refund_returned_to_credit` | payer | 2500 D; 2110 C, 2140 C | `refund_lot` row | operator |
| `external_refund` | payer | 2110 D, 2120 D, 2130 D, 2140 D, 4190 D; 1100 C | `processor_payment_state`; ext `processor_refund` | system |
| `dispute_freeze` | payer | 2110 D, 2120 D; 2140 C | `dispute` row; ext `processor_dispute` | system |
| `dispute_withheld` | platform | 1300 D, 5400 D; 1100 C | `dispute` row | system |
| `dispute_won` | payer | 1100 D, 2140 D; 1300 C, 2110 C, 5400 C | `dispute` row | system |
| `dispute_lost` | payer | 2140 D, 2500 D, 5500 D; 1300 C, 2110 C (the release of the frozen remainder, 14.2) | `dispute` row | system |
| `chargeback_recovery` | session | 2130 D; 5500 C | reservation row on a `charged_back` lot | system |
| `promotional_expiry` | payer | 2210 D; 3910 C | lot row | system |
| `promotional_forfeit` | payer | 2210 D; 3910 C | lot row; closing account | system |
| `rounding` | payer | 2110 D; 5700 C | lot row; closing account | system |
| `supplier_exposure_write_off` | platform | 5200 D; 2310 C; exposure close | attempt | system |
| `supplier_invoice_match` | platform | 2310 D, 2300 D, 5200 C, 5700 D/C; exposure close | `supplier_invoice` row; ext `supplier_invoice` | system |
| `supplier_payment` | platform | 2300 D; 1000 C | ext `bank_transaction` | operator |
| `supplier_prepayment` | platform | 1200 D; 1000 C | ext `bank_transaction` | operator |
| `manual_adjustment` | platform | **only** 2300, 2310, 2400, 3000, 4100, 4110, 4190, 5100, 5110, 5200, 5400, 5500, 5700, either side. Barred: all of 1xxx, 21xx, 2500, 22xx, 4200, 5300 and the Exposure book | case; reason | operator, approver above threshold |
| `reversal` | stream of the original | the exact negation of a reversible kind's lines (16.17) | `reverses_entry_id` | operator, approver above threshold |

- A `purchase` posts only against a `processor_payment_state` row, and a `trial_grant` only against a
  `trial_grant` row, so credit cannot be minted without the evidence that earns it.
- `goodwill_grant` is the only way an operator creates customer credit, and it is promotional.
- Above `manual_adjustment.second_approver_above_micro_dollars` (default off for the solo pilot)
  `post_entry` requires an approver; the approver is never the actor (a CHECK and the function).
- Currency: a line's currency is its key's; the balance check groups by it; the key trigger ties the
  key to its lot's currency (17.1 item 5).
- Entry kinds are append-only configuration of the schema: a new kind is a new migration with its
  rows, reviewed like a chart change.

### 17.7 Tamper evidence

The controls above stop the application and its roles. They cannot stop a database administrator,
and a trigger cannot protect against someone who can disable triggers. So the design prevents what
it can and **detects the rest**:

1. **Prevention.** The runtime and service roles cannot post except through the allowlist; they
   hold no ownership, so they cannot `DISABLE TRIGGER`; they cannot `SET session_replication_role`
   (the setting is superuser-only and the role is granted no `SET` on parameter), cannot `TRUNCATE`
   (privilege revoked and the statement-level trigger of 17.1), cannot `COPY` into a table. The
   catalog walk of 26.2 proves each of these against the live roles, and a mutant removes each
   control and the test must fail.
2. **A hash chain per stream, with a versioned hash.** Each stream (a payer, a session, a campaign,
   an account, or the platform) has a head row `(last_seq, last_hash)`. `post_entry` reads it under
   its lock, sets the entry's `stream_seq = last_seq + 1`, `prev_hash = last_hash`,
   `entry_hash_version` = the current version, and
   `entry_hash = SHA-256('df-ledger/' || version || prev_hash || canonical_v(entry))`. **Version 1
   canonical bytes** are length-prefixed, in this order: `entry_id`, `event_id`, kind, business key,
   external reference kind and value, stream kind and id, `stream_seq`, `recorded_at_ms`,
   `effective_at_ms`, `period_id`, `corrects_period_id`, `reverses_entry_id`, **the correlation
   columns `campaign_id`, `session_id`, `attempt_id`, `reservation_id` and `price_version_id`** (each
   encoded with a presence byte, so an absent id and an id of zero bytes differ), reason, case
   reference, actor kind and id, approver id, and then the lines in line order, each as the key's
   dimensions (not the surrogate `key_id`, so a restored database hashes the same bytes) and the
   signed amount. The correlation columns are in the hash so that neither the chain nor
   `journal_compare` can miss an entry being re-attributed to another attempt, session, campaign or
   price version. That list is frozen now, while version 1 is still unwritten: a later entry field is
   a version 2 with its own function, added beside version 1 (17.6, "Entry hash versions"), and
   every verifier uses the version stored on the entry. A stream is locked by the transaction that
   already holds its anchor (payer, session, campaign or the screening account), so the chain adds no
   contention (19.2). Only the `platform` stream is shared, and only low-rate entries (fees, payouts,
   supplier payments, write-offs, manual adjustments) go to it.

   **What the hash does not cover.** The hash covers entries and their lines and the correlation
   ids that tie an entry to an attempt, session, reservation or price version. **The subledger row
   after-images** (lots, allocations, reservations, shares, attempts, usage events, receipts,
   refund and dispute rows) **are not hash-covered**: an edit to such a row does not change any
   entry hash. They are protected by other controls, each named here: (a) the commit-time triggers
   of 17.3, which tie reserved and allocated keys, lot totals and states to the keys on every
   commit; (b) the per-kind equations of 17.6, which bind each entry's lines to the rows of the
   same event; (c) the daily subledger and conservation reconciliation (21.6, 21.7), which
   recomputes every row's totals from the hash-covered lines; (d) the post-restore replay check
   (22.3), which compares every row with a model built from the journal; and (e) the journal
   itself, whose sealed after-images are the reference the replay check uses. A forged subledger
   row that disagrees with the lines is therefore detected by (a), (c) or (d); one that agrees with
   them is, by construction, not a money difference.
3. **The daily DB-versus-journal job** (`journal_compare`, 21.9). Every finance event reaches O2's
   sealed recovery journal with its entry hashes (22.1); the control plane's journal key is not
   available to the database. The job compares **per stream, by stream position (`stream_seq`),
   never by `global_seq`**, which is not commit-ordered and would skip or repeat entries. It reads
   the confirmed journal tail by stream through O2's reader (22.3: `journal.confirmed_tail`),
   starting from its stored per-stream position, and checks: each journal event for the stream is
   in the database at the same position with the identical hash (recomputed with the entry's stored `entry_hash_version`, never the current one) and chain; the database holds
   nothing beyond the journal's confirmed position of that stream except entries younger than
   `recovery_journal.maximum_unconfirmed_seconds` (read with `journal.oldest_unconfirmed_age`);
   the chain has no gap. A rewritten, deleted or inserted entry changes a hash or breaks a chain
   and is reported as an alert difference. The run records its watermark as the confirmed journal
   sequence and a digest of the stream heads (17.5).
4. **The daily controls job** (`integrity_controls`). Against a manifest compiled into the
   binary from the migrations (so the database cannot supply its own answer) it checks: every
   expected finance trigger exists and `tgenabled = 'O'`; no unexpected trigger or rule exists on a
   finance table; the owner and `relacl` of every finance table and function match; each definer
   function's `prosecdef`, `proconfig` (fixed `search_path`) and the SHA-256 of `prosrc` equal the
   manifest; `pg_db_role_setting` sets no `session_replication_role` for any finance role; no role
   other than `df_finance_definer` has write privileges; the append-only and TRUNCATE triggers
   fire (a probe in a rolled-back savepoint of a scratch table). It also runs after every restore.
5. **Residual risk, stated to the owner.**
   - *Database administrator.* A superuser who can alter the database *and* obtain the journal key
     and the Backups credential held by the control plane can rewrite history and re-seal it. The
     sealed journal and the owner-held off-host watermark (O2 13) bound this: entries older than the
     last watermark cannot be altered undetected, and an alteration since the last watermark is
     detected at the next one unless both stores are compromised. The managed database's
     administrator remains trusted for availability and for the time between the daily jobs.
   - *Self-written anchors.* The evidence rows an entry kind requires, `processor_payment_state`
     and `trial_grant`, reach the database through the same service channel as the entries
     (`apply_processor_state` from B, `grant_trial` from B's verification). The allowlist and the
     anchor rule stop a defect from posting *without* evidence, but a defective or compromised
     service (F or B) can forge the evidence itself, for example a captured payment that never
     happened, and mint purchased credit within the allowlist. This is a residual risk, not
     something the database can prevent. The controls are detective and bounding: (1) the daily
     processor reconciliation compares every purchase with the processor's own list of charges,
     exactly, in whole cents, so a purchase with no real charge is a difference the next morning
     (21.2, with its planted-fault test); (2) the purchase risk limits ($100 per purchase, $200
     outstanding, $200 per day for a new payer) cap what a forged purchase can mint per payer
     (7.3); (3) one trial per account and per email key; (4) the hash chains and the journal
     compare make a later edit of the evidence visible. A signature by B over each `PaymentState`
     that F verifies is a possible later hardening, not designed here.

### 17.8 Accounting periods, the posting date and the close procedure

- **Time zone and cut-off (ruling (c)).** `accounting.time_zone` is `UTC` by default and
  configurable. The accounting date of an entry is the **posting date**: its `period_id` is derived
  inside `post_entry` as the period containing `recorded_at_ms`, the database clock, rendered in the
  configured zone. The caller supplies nothing: it cannot pick an open period, and a late item (a
  fee, a payout, an invoice, a dispute outcome) lands in the period in which it is posted and
  never back-dates. `effective_at_ms` is the business time and is information, constrained to be
  no later than the recording time plus one day. Changing the zone is allowed only from the first
  instant of a future period (`accounting_time_zone_history (effective_from_ms, time_zone)`, DDL in 17.1; the zone in force at `recorded_at_ms` is the latest row at or before it), so no
  month can be split.
- **Periods are pre-created.** `accounting_period` (DDL in 17.1: `period_id` yyyymm, `starts_at_ms`, `ends_at_ms`,
  `status`, `closed_at_ms`, `closed_by`, `reviewed_by`, `digest`). Migration 0020 creates 24 months; the
  daily job `period_precreate` keeps `period.precreate_months_ahead = 12` months ahead and alerts
  if fewer than three remain, so the foreign key never fails the first entry of a month.
- **Closed is final.** A trigger allows only `open -> closed` and refuses any other change to a
  period row, including a delete or a reopen; closed periods are never reopened. A mistake in a
  closed period is corrected in the open period by a reversal or an adjustment that records
  `corrects_period_id` (a column of `journal_entry`, 17.1), which the adjustment listing shows.
- **The close procedure.** `finance.close_period(period_id, closer, reviewer)` (operator
  permission `FinanceClose`; definer) refuses unless: the period's end has passed; the closer is
  not the reviewer; a `reconciliation_run` of kind `full` whose `as_of_ms` is at or beyond the end of
  the period and whose journal watermark covers every event recorded before `as_of_ms` exists, its outcome is `clean` or every difference is `within_tolerance`, `explained`
  or `corrected`, and a `reconciliation_signoff` by the reviewer exists; the recovery journal has
  confirmed every event of the period (no pending entry older than the period end); every earlier
  period is closed. It takes the **per-period** advisory lock exclusively: the key is `hash(finance.period, period_id)`, every posting holds the shared lock of the period it posts into, and a posting always lands in the current period (its period is the one containing the database clock), so closing an ended period never waits for or stalls current postings; an in-flight posting into the closing period (one that started before its end) finishes first, and later ones see `closed` and are refused.
  It then writes the immutable
  `period_balance` snapshot (opening, debits, credits, closing for every key with activity or a
  non-zero balance, from the previous snapshot plus the period's lines) and the period digest, and
  sets the period `closed`. Exports of a closed period regenerate byte for byte (23.5).

### 17.9 Roles and grants (also 0024, in O1's 0018 role model)

`df_finance` (control-plane finance service; `SELECT` on finance tables and views, `EXECUTE` on
`lock_flow`, `commit_event` and the specific definers below; **no DML**; no `replay_event`, and no membership of the replay role), `df_finance_definer`
(NOLOGIN, owns the tables, triggers and definer functions), a read-only `df_finance_support`
(redacted views for support, plan 9.13). Granted to O1's and the control plane's roles only as
`EXECUTE` on the narrow definer functions:

| Role | Functions | Caller check |
|---|---|---|
| `df_campaign_runtime` | `meter_funded_until`, `session_funding_state`, `prepare_dispatch`, `cancel_prepared`, `record_usage` | `df_runtime.leased_campaign()` equals the attempt's or session's campaign |
| `df_egress` | `mark_attempt_sent`, `mark_attempt_unsent`, `record_transport_outcome` (each with an account-attempt variant for content screens, 11.1) | `df_runtime.current_dispatcher()` (O1 10.2 item 6); for `mark_attempt_sent` also `dispatcher_serves_campaign` |
| `df_control` | `attempt_status`, `cancel_unclaimed_effect`, `attempt_outcomes_after`, `start_readiness`, `open_session_funding`, `end_session_funding`, `personal_usage_statement`, `session_funding_state`, `prepare_dispatch_account`, `cancel_prepared_account` | the control plane's own role; no placement |
| `df_finance` | everything else, including `reconstruct_campaign`'s final call to O1's `record_money_state`, and `record_replay_check` (the independent check of 17.6; not granted to the replay role) | service role; every transaction begins with `enter_service()` |
| `df_finance_replay` (a **dedicated LOGIN role** with its own credential and `CONNECTION LIMIT 2`; not reached by `SET ROLE`, and `df_finance` is not a member) | `replay_event`, `replay_begin`, `replay_complete`, `rebuild_work_items`; and O1's `df_runtime.restore_complete(recovery_epoch)` | refused unless `df_runtime.service_state()` is `restoring` and `replay_state.status` is a replay status (17.6); `replay_complete` additionally refuses unless a passing `replay_check_record` with the live stream-heads digest exists; `replay_begin`, `replay_complete` and `rebuild_work_items` write only through `apply_change` as unjournaled changes |

No finance table is granted to a runtime role, so O1's catalog walk (O1 4.4 item 4) finds none
needing a policy. The test that no finance table is writable by any role is a positive assertion
about the exact grant list above, not an empty-set check (26.2).

## 18. API contracts

`df_finance::FinanceService` (native, async, every call under a caller deadline). Requests carry an
`OperationId` for every mutation (plan 15.2); a timeout returns `Unknown { operation }` with the
lookup method. Errors are `FinanceError` variants a caller can branch on: `Refused(..)`,
`Conflict`, `NotFound` (indistinguishable from another tenant's id), `StoreUnavailable`,
`ServiceHalting` (retryable, from `DF002`), `Unknown { operation }`. Principals come from the
authenticated session or the placement identity, never a form field. A `Display` principal is
refused by every command (AUTH-02).

### 18.1 Admission (called by the runtime through the finance endpoint, by the control plane for screens)

```rust
pub enum AdmissionPrincipal {
    Campaign { tenant: TenantId, campaign: CampaignId, fence: OwnerFence },  // O1's three parts
    Account { account: AccountId },                  // fence-free; Payer::Company(ContentScreen) only
}
pub struct AdmissionRequest {
    pub operation: OperationId,            // the runtime's effect operation id
    pub attempt: AttemptId,                // minted by the caller once (D22); part of the fingerprint
    pub effect: EffectId,
    pub principal: AdmissionPrincipal,
    pub payer: Payer,                      // Table(SessionId) | Company(CompanyClass)
    pub route: ProviderRoute,              // closed (C's route table)
    pub work_class: WorkClass,             // Essential | Optional | Premium
    pub maximum_units: Vec<(Category, u64)>,  // must cover every category the route declares
    pub estimate_units: Vec<(Category, u64)>,
    pub quote: Option<QuoteId>,            // single use
    pub media: Option<MediaAdmission>,     // O2's admit_generation inputs
    pub request_digest: [u8; 32],          // canonical request, never the prompt
}
pub enum AdmissionOutcome {
    Admitted { reservation: ReservationId, attempt: AttemptId, retail_maximum: Money,
               supplier_maximum: Money, price_version: PriceVersionId },
    Refused(AdmissionRefusal),             // section 11.2; retryable() says whether a later retry can differ
}
```

Where it runs (D30, settled): the control plane's internal mTLS listener beside O2's media gateway,
authenticated by the placement certificate (the SAN must equal `fence.placement_id` and the
placement must be current, O1 10.1), so no runtime ever holds finance write rights. Fingerprint =
canonical encoding of every field above.

### 18.2 Attempt progress and settlement facts (no live owner needed except prepare and send)

| Call | Caller | Effect |
|---|---|---|
| `prepare_dispatch(attempt, fence)` (SQL definer) | runtime effect claim | `Reserved -> DispatchPrepared` |
| `cancel_prepared(attempt)` (SQL definer) | the claiming runtime | `DispatchPrepared -> CancelledUnsent`, release |
| `prepare_dispatch_account(attempt)`, `cancel_prepared_account(attempt)` (SQL definers) | O2's screen pipeline in the control plane (`df_control`) | the same arrows for an account-principal attempt (11.1); no fence, no placement |
| `mark_attempt_sent(attempt, fence)` (SQL definer) | dispatcher | 11.4 item 2: `DispatchPrepared -> Sent`; a null fence only for an account-principal attempt |
| `mark_attempt_unsent(attempt)` (SQL definer) | dispatcher | `Sent -> VerifiedUnsent`, release in the same call |
| `cancel_unclaimed_effect(tx, attempt)` | O2 sweeper, user cancel | `Reserved -> CancelledUnsent`, release; `Cancelled \| AlreadyClaimed` |
| `record_transport_outcome(attempt, outcome, provider_units)` | dispatcher, in the dispatcher's own transaction | `ResultKnown` / `OutcomeUnknown`; checks `current_dispatcher()`; carries the provider-returned usage units (12.1) |
| `record_usage(attempt, Usability, runtime_units)` | runtime (C's `BilledCall` in hosted mode) | usability and any units the provider did not return (`Estimated`, 15.3); units only; enqueues `settle` |
| media `FinalizeOutcome` | F's idempotent consumer of `media.settlement_outbox` | usability for media (12.6) |
| `attempt_status(attempt)` | O2 sweeper | the mapping of 11.5 |
| `attempt_outcomes_after(after_seq, since_ms, limit)` | O2 consumer | the outcome events of 12.6 |
| `settle(attempt)` | F's settlement worker from a `settle` work item | one settlement transaction per attempt |

### 18.3 Release and session end

`release(ReleaseRequest { operation, reservation, reason: UserCancelled | NotDispatched | ... })`;
`withdraw_allocation(WithdrawRequest { operation, allocation, amount: All | Exactly(Money) })`;
`end_session_funding(session)` (13.4: sets `settling`, moves allocations to `closing`, releases their
unreserved parts, leaves holds in place, refuses new work; the finish transaction later calls
O1's `settlement_finished`). The statement is provisional while any attempt is unresolved
(plan 9.11).

### 18.4 Credit

`credit(CreditRequest { operation, kind: ReverseCharge { usage_event, amount, reason } |
Goodwill { payer, amount }, actor, case_ref, approver })`; operator permission required; a second
approver above `manual_adjustment.second_approver_above_micro_dollars` (default off for the solo
pilot, plan 7.3; question 28.8); the approver is never the actor. Semantics: 12.5 and 14.4.

### 18.5 Quote

`quote(QuoteRequest { session, route, work_class, maximum_units, estimate_units, request_digest,
requested_by })` -> `QuoteView` (customer-safe) and stored quote; `approve_quote(operation, quote,
allocation, decision)`. A quote is single-use (9.3).

### 18.6 Processor-facing (B)

`apply_processor_state(PaymentState { processor, payment_ref, charge_ref, purchase_order_ref, payer,
currency, captured, captured_micro_dollars, refunded_total_micro_dollars, refunds: [(refund_ref,
amount)], dispute: Option<DisputeState>, fee_micro_dollars: Option, tax_evidence, object_revision })`
-> `Applied { lot, posted: Vec<EntryKind> } | AlreadyCurrent` (7.3: serialized on the payer and
payment-state rows; never raises on a stale state); `observe_refund_lot(refund_lot, outcome,
processor_refund_ref, object_revision)`; `purchase_limits(account, amount)`; `grant_trial(tx,
account, email_grant_key)`; `set_payer_standing(tx, account, standing)`; `closure_blockers(account)`
(purchased credit of a cent or more, open holds, pending refunds, open disputes; a sub-cent remainder
does not block), `release_account_allocations`, `round_closing_remainder`, `forfeit_promotional`
(14.5), which B's closure and deletion paths call.

### 18.7 Funding and policy

`allocate(AllocateRequest { operation, session, payer, amount, consent })`,
`update_session_policy(PolicyRequest { operation, session, expected_revision, policy })` ->
`Updated { revision, excluded_payers } | ConsentRequired { payers }`. The internal `funding`
consumer applies S1's feed through `funding_feed_position` and `funding_effect` (17.2).

### 18.8 Session lifecycle hooks (O1, S1)

`open_session_funding(session, campaign, tenant)` (idempotent; pins the current price version;
called by S1's Start before O1's `Start`, finance first), `start_readiness(session)` -> typed
blockers (`NoFunding`, `SponsorConsentMissing`, `EssentialStartupNotFundable`, `ConsentIncomplete`,
`PaidWorkFrozen`) for S1's `FundingReadiness` and O1's Start check, `session_funding_state(session)`
-> `Funded | Low | FundingRequired` (lock-free read; `NOTIFY df_finance_funding` is a hint),
`meter_funded_until(session)` (13.1), `end_session_funding(session)` (13.4),
`reconstruct_campaign { tenant, campaign, fence, reconstruction_id }` (22.2, an endpoint on the
internal listener, not a SQL grant).

### 18.9 Views: stable shapes for P (D28) and B, customer-safe by type (D26)

Types live in `df_commerce::views::customer` and `df_commerce::views::operator`, two modules with
no shared types and no import of one from the other (pure, serde, wasm-safe). Each customer view has
a schema constant (`dungeonflux.finance.<view>/1`), closed enums in `snake_case`,
`deny_unknown_fields` on requests, and additive-only evolution (a breaking change is a new schema
number served side by side). Money fields are `MoneyView { micro_dollars: u64, display_cents: i64 }`
with the display rule of 4.8.

| View | Fields (customer-safe) |
|---|---|
| `BalanceView` | purchased available, promotional (trial) available, allocated to sessions, reserved for pending work, consumed this period (plan 9.7), currency, `proposed_pilot` |
| `SessionFundingView` | price version label (opaque) and effective date, active preset or custom ceiling, session ceiling, pot available, open holds, spent, runway range (at selected maximum, at recent rate), funds state, the caller's own allocation, authorization remaining, consent status per payer (names and amounts only when `contributions_visible`), end-of-session treatment |
| `BudgetChipView` | "Pot", "Held", hourly maximum (plan 9.10); a display-safe variant without payer details |
| `QuoteView` | output type, estimate, maximum ("Generate for up to $X"), who pays (own share estimate), expiry, latency range when known, cancellation policy code, price version label |
| `StatementView` | lines: category label (closed vocabulary), `at_ms` (client renders local time), session and campaign, `quantity_text` (closed formatter, below), price version label, final retail charge, reference, status (`settled`, `pending_hold`, `credited`, `technical_failure_credited`, `released_not_charged`); totals; provisional flag (plan 7.2, 9.11) |
| `TransactionListView` | purchases, allocation and release, usage, credits, refunds, disputes, adjustments, distinctly typed (plan 9.7); cursor paging scoped to the caller |
| `RefundPreviewView` | lots, amounts, released allocations, pending holds, tax, card suffix, manual-payout flag (plan 6.7) |
| `PurchaseStatusView` | pending, succeeded, failed (B's state) and the resulting lot once posted (UX-02) |

**Quantity plus charge per line (ruling (f)).** D26's "actual charges per line" is read as
permitting `quantity_text` beside the exact charge, so a customer can divide one by the other and see
an effective rate. That is intended and configurable (`statement.show_quantity = true`); it is not a
leak of the table fee, markup, provider or supplier price, which no view has a field for.

**What makes "no fee, markup, provider or cost" true by construction and by test.**

- *Types.* Customer views have no field for the table fee, the markup, a provider name, a provider
  price, a supplier cost or a margin. The operator module holds those; the customer module imports
  nothing from it (a source scan test).
- *No free text.* Every string in a customer view is a closed enum code, an id, a timestamp, a
  schema constant, or produced by a closed formatter: `quantity_text` is `{n}{unit}` composed from
  a numeric quantity and a unit from a closed set, the category label comes from a closed table,
  and the price version label is **opaque**: a short code derived from the version id by
  hashing (for example `pv-7k3f2`), never a date range, a rate or a name an owner typed.
- *Allowlist golden schema.* Each customer view has a checked-in golden schema listing every
  permitted field, its type and, for strings, its closed vocabulary or pattern. A test serializes
  every customer view from fixtures and fails on **any field not on the list** (not on a list of
  forbidden words), so a new field cannot slip through by having an innocent name.
- *Value scan.* The fixtures use distinctive prices: a markup of 7/4, a platform rate of $2.37 per
  hour, tariff rates with unusual digits and a provider and model with a distinctive name. A test
  serializes every customer view across every fixture and fails if any of those numbers (as
  `1.75`, `7/4`, `2.37`, `2370000`, their reciprocal and rounded forms), the provider or model
  names, or the supplier cost digits appear in any key **or any value** of any output, and requires
  every number outside the allowlisted money fields to be an id, a count or a timestamp.
- Mutants (26.5): add a `label` string field; put the markup in `quantity_text`; put a provider name
  in the version label; each must fail the golden or the value scan.

Operator views (operator API only, permission `FinanceInspect`): `FiveBalancesView` (6), the
treasury view, `OperatorBreakdown` per attempt, session, payer or period (table fee portion,
provider units, pinned tariff, markup, supplier actual and maximum, margin, company loss and class,
payer shares, all three attempt states, provider request reference, fence), reconciliation runs and
differences, company loss by class, forgone revenue, unknown exposure aging, content screens per
account, export downloads.

## 19. Concurrency and locking (D25, D29)

### 19.1 Lock order and isolation

**Isolation.** Every finance transaction runs at `READ COMMITTED` with explicit row locks. Each
statement sees a fresh snapshot, so a read taken after a lock is acquired sees the committed
state the lock protects. Correctness comes from the locks, the checks and the constraints;
`SERIALIZABLE` is not used (it would turn the hot paths into retry loops). The role sets
`lock_timeout = 2 s` and `idle_in_transaction_session_timeout = 5 s` (O1 6.1) and
`transaction_timeout` <= 10 s (S1's shared role rule); a timeout is a retryable typed refusal
(`StoreBusy`), never a partial write.

**Across owners (D25, D29).** `enter_service()` first (shared advisory lock, O1 6.1); then **finance
rows**; then O1 directory and session rows (`fence_current` takes `FOR SHARE` on `service_state`
and `campaign_owners`); then S1's `campaigns.campaign` and child rows (only S1's own transactions);
then `media.quota_usage`, `media.asset` by id, and reference and grant rows. A finance flow never
locks a B accounts row or an S1 row, and a transaction that starts in media never afterwards locks
finance rows.

**Inside finance, always ascending by this rank, ids ascending inside a rank:**

| Rank | Rows | Taken by |
|---|---|---|
| 0 | `work_item` claim row (`SKIP LOCKED`); nothing waits behind it | queue workers |
| 1 | `payer`, by account id | allocate, withdraw, refund, dispute, credit, lot closing, `processor_payment_state` flows, closure |
| 2 | `processor_payment_state` by (processor, payment ref) | purchase, refund and dispute state |
| 3 | `session_funding`, by session id | admission, settlement, release, meter, session end |
| 4 | `quote`, by id | admission with a quote, quote approval |
| 5 | `allocation`, by id | all flows that touch allocations |
| 6 | `funding_lot`, by id | **`FOR UPDATE`, ascending, in every flow that can change a lot row** (a total such as `consumed`, `refunded`, `extinguished`, `rounded`, a state): settlement, meter charge, **`release`, `company_loss`, `chargeback_recovery`** (a release into a `ChargedBack` lot posts a recovery and raises `extinguished`, and the lot state is known only after the lock is taken, so these flows cannot start with a share lock), credit, refund, dispute, rounding, forfeiture, lot closing. **`FOR SHARE` only in flows that provably write nothing to the lot row**: admission, quote checks, the funding-feed release of unreserved credit. A flow decides its mode when it takes the lock and never upgrades a share lock it holds; a flow that cannot tell in advance takes `FOR UPDATE` |
| 6b | `company_exposure_counter`, by (scope, period) | admission with a configured company limit; after the lots, before the operation rows; absent when every limit is unlimited |
| 7 | operation rows, in this table order, ids ascending: `provider_attempt`, `reservation`, `reservation_share`, `usage_event`, `refund`, `refund_lot`, `dispute`, `operation_receipt`, `funding_feed_position`, `funding_effect` | every flow that changes one; **existing-attempt flows lock the attempt here, after the session, allocations and lots, never first** |
| 8 | `ledger_stream` heads (`payer`, `session`, `campaign`, `account`, `platform`), by (kind, id) | `commit_event` |
| 9 | `account_balance`, by key id | `commit_event`, all of them before any other write step |
| P | accounting period, advisory lock **keyed per `period_id`**, shared by every posting into that period, exclusive for its close (a close of an ended period never waits for current postings) | `commit_event`, `close_period` |
| then | O1 (`fence_current`), then media | admission, send, cancel |

Why this is deadlock-free, flow by flow:

- **Admission** (company or table): ranks 3, 4, 5, 6 (share), 7 (new rows; no waits), 8, 9, **then**
  `fence_current`, then `admit_generation`. The entry is posted (ranks 8 and 9) before the fence and
  media steps; a refusal there rolls everything back, which releases the balance locks. The
  earlier design posted after the media step, so the balance rows came after the media quota row,
  while O2's `cancel_unclaimed_effect` and F's 24 h resolution locked balances and then quota: a
  cycle. The order is now finance, then O1, then media in every flow.
- **Flows that begin from an attempt** (settlement, release, cancel, credit, `mark_attempt_unsent`,
  deadline items, the O2 cancel): resolve attempt -> session and payers with an **unlocked read**,
  lock ranks 1-6 in order, then lock the attempt-level rows (rank 7) and **re-check the state**; if it
  changed, do nothing. Workers claim from a `work_item` queue row (rank 0), never from the attempt
  row. A user `release` (session first, then attempt) and a settlement (the same) therefore
  agree.
- **`mark_attempt_sent`** locks only the attempt (rank 7) and then calls `fence_current`; it takes no
  finance row above rank 7 and posts nothing, so it cannot form a cycle with a user release.
- **Lots.** The lot row carries running totals (`consumed`, `refunded`, `extinguished`, `rounded`) that
  settlement, meter charge, credit, refund and the dispute flows must write, and the commit-time
  conservation check (17.3) depends on them being current. So every flow that writes a lot total
  takes the lot **`FOR UPDATE`, directly, in ascending lot-id order**, never `FOR SHARE` first. Two
  sessions that share a payer's lot, settling at once, queue on the lot row: both hold their own
  session row (rank 3) and wait on the lot (rank 6) in the same order, so there is no cycle and no
  lost total (the `shared_lot` test of 26.2 runs them concurrently). Admission, which reads the
  lot's state and writes nothing to it, takes `FOR SHARE`; a settlement waits for admissions that
  hold the share, and an admission waits for a settlement that holds the update lock, and neither
  upgrades. Closing or reopening a lot (`Closed`) happens only in `close_settled_lots(payer)` and in
  the credit and refund-return flows, which hold the payer row; state-changing flows take the payer
  row first, which also serializes them against each other. A flow that would derive `consumed` from
  allocation totals instead was considered and rejected: it makes the conservation check a join over
  every allocation of the lot.
- **Balances** are locked together, in key-id order, at the start of `commit_event`, so two entries
  over the same keys lock them in the same order.
- **Funding feed effect** (the `funding` consumer applying a `left` or `removed`): claimed from a
  `work_item` (rank 0), it resolves the payer's allocations with an unlocked read, takes the payer
  (1), the session rows (3), the allocations (5), the lots (6, `FOR SHARE`: the release moves credit between keys and writes no lot
  total), then writes the effect
  and advances `funding_feed_position` (rank 7, last of the table order). The first allocation of a
  payer in a campaign inserts the position row (`ON CONFLICT DO NOTHING`, 17.2) in rank 7 too; the
  insert never waits on a row lock, only on an uncommitted conflicting insert, which belongs to a
  transaction that holds no lock the inserter's transaction needs.
- **Receipt serialization.** The receipt is read inside `lock_flow` after the locks; with no
  session row the unique key on the receipt and on `attempt_id` is the serialization point and a
  violation is mapped to the stored outcome (11.2 step 2).
- **Streams** are locked at rank 8 by the same transaction that holds the anchor (an `account` stream by the screen admission of that account alone); the `platform`
  stream is shared only by low-rate entries.
- **Dispatcher send** locks the attempt (rank 7), then O1. **O2 sweeper cancel** runs
  `cancel_unclaimed_effect` first in its transaction (ranks 3-9 as above), then locks media quota and
  asset rows.
- Finance tables carry no foreign key into another owner's schema, because a foreign-key check
  takes `FOR KEY SHARE` on the referenced row, and S1 takes `FOR UPDATE` on `campaigns.campaign`
  for every event-writing change; cross-owner ids are validated when the finance row is created
  instead. O1's segment roll reads finance with no lock. `member_snapshot` is a plain read.

**Enforcement.** The store's `lock(rank, id)` helper keeps a per-transaction cursor and refuses (a
debug assertion in tests, a counter and an alert in production) a lock request at a lower rank than
one already held. The deadlock test of 26.2 drives every flow concurrently and asserts no `40P01`.

### 19.2 What serializes what

- Admissions of one table serialize on its `session_funding` row (about 10 ms each). POT-01's six
  concurrent requests queue there; the nonnegative balance checks are the backstop.
- Different tables never contend: hot shared accounts (revenue, expense, clearing, offsets,
  exposure) keep no balance row (section 5), the exposure accounts included (9100 is unenforced), and
  each ledger stream is locked by the anchor the flow already holds, so settlements of many tables
  do not serialize on one row.
- A payer's own operations serialize on their `payer` row; an admission never takes it, and every
  flow that takes both takes `payer` first, so no cycle exists.
- The platform stream is the one shared chain head. Posting to it is low rate by construction (17.6).

### 19.3 Async ownership

All workers (settlement and deadline workers on `work_item`, the outcome consumers, funding feed
consumer, meter holds and pricer, journal-replay watcher, reconciliation and integrity schedulers,
period pre-creation and close, export builder) run in one `JoinSet` owned by `FinanceService`,
stopped on the composition's shutdown signal; nothing is detached. Queues are bounded
(`FINANCE_WORK_QUEUE = 1024`, a full queue refuses with a typed busy outcome and one audit line).
Every database call has a deadline. Workers claim work with `FOR UPDATE SKIP LOCKED` and a lease
plus attempt generation, so a delayed completion from an old claim cannot overwrite a newer one
(plan 6.6). Every F process is bound by the reconnect gate (22.2). The replay job is one task with its own connection pool under its own login role, `df_finance_replay` (17.6); the live service never holds that role.

### 19.4 Payer standing mirror

B decides standing (restriction after a dispute or by an operator). B's restrict and unrestrict
transactions call `set_payer_standing(tx, account, standing)` first (finance rows first). Admission
and allocation read the mirror under the locks they already hold. A restriction stops new
allocations and admissions funded by that payer; their in-flight holds settle normally.

## 20. Idempotency keys

`business_key` = SHA-256 over a domain tag and the fields below, length-prefixed (the same
canonical encoding as O2's `MediaRequestKey`), unique with the entry kind.

| Operation | Idempotency key | Repeat behaviour |
|---|---|---|
| Purchase posting | (processor, payment_ref) | `AlreadyCurrent`; a new event id changes nothing (PAY-02) |
| Fee, payout | (processor, balance transaction ref) | no-op |
| Trial grant | account_id; email_grant_key | refused, audited |
| Goodwill grant | (case_ref, operation_id) | first result |
| Allocate, raise | (payer, operation_id) + fingerprint | first result; changed fields `Conflict` |
| Withdraw | (allocation, operation_id) | first result |
| Policy update | (session, expected revision, operation_id) | first result; stale revision refused |
| Quote | (session, request_digest, price_version) while open | same quote |
| Quote approval | (quote, allocation, operation_id) | first decision |
| Quote use | `reservation.quote_id` unique | a second admission is `QuoteAlreadyUsed` |
| Admission | (principal, operation_id) + fingerprint including `attempt_id`; `attempt_id` unique. **Only an admitted operation has a receipt; refusals are never stored** | stored outcome (SAVE-02); a refusal is re-evaluated on retry |
| Prepare, send | attempt state guard | refused if not in the source state |
| Transport outcome | (attempt, outcome kind) | first wins; a contradicting second is an alert |
| Usage report | attempt_id | first wins; a contradicting second is an alert |
| O2 outcome feed | `settlement_fact.attempt_id` | no-op on a re-read (idempotent rescan, 12.6) |
| Settlement | attempt_id (`usage_event.attempt_id` unique) | no-op (JOB-02) |
| Release | reservation_id | no-op |
| Customer credit | (usage_event, operation_id); `reversed <= retail` under the row lock | first result |
| Meter hold | (session, interval_seq) | no-op |
| Meter charge | (session, segment_seq) | no-op (TIME-03) |
| Refund request | (payer, operation_id); processor key = refund_lot_id | first result; one processor call (REF-02) |
| Refund outcome | (refund_lot, processor_refund_ref, object_revision) | monotonic |
| Dispute | (processor, dispute_ref, status) | monotonic |
| External refund | (processor, refund_ref) | no-op |
| Company loss | (attempt_id, loss_class) | no-op |
| Manual adjustment | (case_ref, operation_id) | first result |
| Manual refund payout | (refund_lot, bank transaction ref) | first result |
| Reversal | `UNIQUE (reverses_entry_id)` | refused |
| Forfeiture, closure rounding | (lot, account closure id) | no-op |
| Funding feed effect | (campaign_id, event_seq) | no-op (17.2) |
| Reconstruction | reconstruction_id | stored outcome (22.2) |
| Restore replay | (recovery_epoch, journal_sequence) and event_id | no-op |
| Export | (period, kind, format); closed periods regenerate identically, open ones carry a watermark | 23.5 |

Receipts are kept 90 days, then their namespace is retired so an old id can never execute again
(B's pattern).

## 21. Reconciliation jobs (plan 7.3, OPS-04)

Owned scheduled jobs (daily at `reconciliation.daily_at_utc = 03:00`, the integrity jobs at
`integrity.daily_at_utc = 03:30`, at startup, and on operator request). Each writes a
`reconciliation_run` and one `reconciliation_difference` per unexplained difference; none of them
posts a balancing plug (plan 7.3). Differences inside a stated tolerance (4.9) are recorded as
`within_tolerance`, so a normal day reports none and the alert means something.

1. **Journal integrity** (`journal_integrity`). Every entry balances per book and currency (the
   constraint guarantees it; the job proves it over the period); every enforced `account_balance`
   equals the sum of its lines; no line references a missing key; every key's lot class and
   currency agree; the hash chain of every stream is continuous.
2. **Processor and bank** (`processor`, `payouts`, `bank`). B's `ProcessorLedgerSource` lists the
   processor's charges, refunds, disputes, fees and payouts for the period (fake in tests). Each
   succeeded charge has exactly one purchase posting; each refund one refund or external-refund
   posting; each dispute its states; payouts equal the clearing reductions; the clearing balance
   equals the processor's reported balance (exact, whole cents). The **bank tie-out**: the imported
   `bank_statement_line`s are matched to `payout` and `manual_refund_payout` and
   `supplier_payment` entries by bank transaction reference; unmatched statement lines and
   unmatched entries are listed.
3. **Supplier usage** (`supplier_usage`). Sent attempts versus C's cost records by attempt id versus
   imported provider invoices (C's bill parser, 29.3), within the tolerance of 4.9; matched exposure
   closed (12.4); invoice spend no attempt explains is `unmetered_route` with an alert.
4. **Reservations, holds and feeds** (`reservations`, `outcome_feeds`). Open reservations, customer
   holds past the release deadline, Reserved and DispatchPrepared attempts past their expiry, Sent
   and Unknown attempts by age, duplicate settlement attempts, refunds pending by age; each with an
   alert threshold. O2's terminal attempts with no `settlement_fact` (the rescan of 12.6) and F's
   outcome events O2 has not consumed.
5. **Meter** (`meter`). Every O1 segment closed and older than the back-date window is priced exactly
   once; priced and credited milliseconds add up to the segment; `charged + forgone` equals the
   cumulative floor.
6. **Subledgers** (`subledger`). `reservation_share` totals equal the reserved keys; allocation
   totals equal their keys; lot states agree with balances (a `closed` lot has zero balances); **no
   non-zero key exists on a closed allocation**; the sum of the 9100/9190 lines equals the
   `ExposureOpen` attempt rows.
7. **Conservation per lot.** original = available + allocated + reserved + frozen + refund payable +
   consumed + refunded + extinguished + rounded; promotional: granted = available + allocated +
   reserved + consumed + expired + forfeited.
8. **Restore** (`campaign_restore`, 22.2-22.3): after the service-wide ordered replay of every stream
   (step A) the subledgers rebuilt from the journal are compared with the live rows (the replay
   check); the per-campaign reconstruction (step B) then only reconciles and ends in
   `record_money_state`. Attempts in `ResultKnown` with no usage past the `usage_missing` deadline
   are listed under item 4.
9. **Tamper evidence** (`journal_compare`, `integrity_controls`): 17.7.

The full run (OPS-04, `full`) is all of them over a period; the operator reconciliation page shows
the latest run and every open difference, with the reviewer's sign-off (17.5, 17.8). Every kind has
a planted-fault test (26.2).

## 22. Recovery journal and restore (plan 13.10, D22, OPS-02)

### 22.1 Submitting finance events, and exactly what a payload may hold

`commit_event` (17.6) inserts one `journal.pending_entry` (O2 13; kind `finance_event`) in the same
transaction as every finance change, with metadata (event id, kind, the stream kind and id of each
entry, the campaign id) that O2 stamps on the pending row so a reader can select by stream and
campaign. The payload is **the whole event**, not only the lines: the entries with their lines and
hashes, and the typed row changes (`RowChange` after-images) of every table the event wrote, so a
replay rebuilds lots, allocations, reservations, shares, attempts and their state arrows, usage
events, receipts, refund and dispute rows, price versions and tariffs, payers and standing,
trial grants, periods, sign-offs and imports with the same `apply_change` that wrote them. (Round
0's journal carried lines only; round 0b journaled only the tables an event wrote without saying
that every writer is an event. Now every writer is one: 17.6.)

**The payload is allowlisted field by field. It holds opaque ids, processor reference ids and a
tax-jurisdiction code, and no address or other personal field** (coordinator ruling, round 0c). The
journal is sealed and immutable and outlives account deletion, so nothing in it may be personal data:

| Part | Fields that may appear |
|---|---|
| Envelope | `event_id`, `event_kind`, **`payload_schema`** (the version of the payload layout, stamped on every event and sealed with it; 17.6), journal metadata (stream kind and id, campaign id) |
| Entry | `entry_id`, `event_id`, `kind`, `business_key` (a hash), `external_ref_kind` and `external_ref` (a processor payment, refund, dispute, balance-transaction or payout reference; a supplier invoice id; a bank transaction reference; a support case id), stream kind, id, `stream_seq`, `prev_hash`, `entry_hash`, **`entry_hash_version`**, `recorded_at_ms`, `effective_at_ms`, `period_id`, `corrects_period_id`, `reverses_entry_id`, `actor_kind`, `actor_id` and `approver_id` (opaque operator ids), `reason` (a closed code), `case_ref` (an opaque id), campaign, session, attempt, reservation and price version ids |
| Line | the key's dimensions: code, currency, payer account id, lot id, allocation id, provider, processor, loss class, **jurisdiction code**, promo source; the signed amount |
| Row after-images | opaque 16-byte ids; closed states and enums; amounts in micro-dollars; revisions; times in ms; digests (`request_digest`, `generation_policy_digest`, `tariff_digest`); the price version id and its opaque label; processor reference ids (`processor`, `payment_ref`, `charge_ref`, `purchase_order_ref`, `dispute_ref`, `refund_ref`); `tax_mode`, `tax_jurisdiction` (a code), `tax_policy_version`, the rate numerator and denominator, `tax_micro_dollars`; route, provider and model names; the fence parts and the dispatcher instance id |
| Never | `billing_country`, `billing_region`, any postal address, name or email, `email_grant_key`, card data or suffix, IP addresses, free text (`reason_text`, case notes, prompts), operator names, payer display names, `provider_request_ref`, `quantity_text` |

**What is rebuilt from B after a restore.** Two lot and grant columns are personal or personal-derived
and are not in the journal: the lot's `billing_country` and `billing_region` and the
`trial_grant.email_grant_key`. After a replay the rows exist with those columns `NULL` and
`evidence_state = 'pending_rebuild'`. F's rebuild job fills them from B's records
(`purchase_billing_evidence(purchase_order_ref)` and `account_email_grant_key(account_id)`, reads B
provides; request to B) as `lot_evidence_rebuilt` and `trial_key_rebuilt` events, and the
reconciliation reports any row still pending. While any `trial_grant` row is pending, `grant_trial`
refuses `TrialKeysRebuilding` (retryable) so an email cannot be granted twice in the gap. The
`tax_jurisdiction` code is enough to reverse tax on a refund; the evidence columns are not needed
to move money. A test compares every payload with the allowlist (a golden list, like the D26
views) and has a mutant that adds `billing_region` (26.2).

### 22.2 Restore: one full ordered replay, then per-campaign reconciliation (coordinator contract (2), O1 6.3)

**Reconnect gate.** Every F process treats a broken database connection as a possible failover. On a
new connection it reads `clock_timestamp()` as `reconnected_at` and writes nothing (admission,
settlement, deadline, meter, reconciliation, export) until `df_meta.service_state.checked_at >
reconnected_at` (O1 6.3); past `failover.check_wait_seconds` it exits. `enter_service()` is the
first statement of each transaction, and a halt maps to `ServiceHalting`.

**The one exemption (ruling, round 0f).** A restored database carries an old `checked_at`, and the
service is `restoring`, so the gate could never open for the replay. The replay role
(`df_finance_replay`) is therefore **exempt from the reconnect gate while the service state is
`restoring`**, in the same way O1's own failover-check transaction is exempt (O1 6.3): it may write
exactly what 17.6 lists for it (`replay_event`, the replay bookkeeping, `restore_complete`) and
nothing else, and `record_replay_check` (the independent check, run by `df_finance`) is exempt on
the same terms for the one unjournaled row it writes. Every other F write, and the replay role in
any state other than `restoring`, stays behind the gate. The exemption is part of the request to O1
(the gate predicate reads `service_state()` and lets these definers through when it says
`restoring`). Test: `the_replay_role_and_the_check_record_write_while_restoring_with_a_stale_checked_at_and_every_other_f_write_is_refused`; mutant: apply the
exemption to `commit_event`.

**Step A: the full ordered replay, once per restore, for all streams.** A payer-stream event spans
campaigns (a payer funds several), and chain order is per stream, so replaying only "the events of
one campaign's payers" would leave missing prerequisites and break chain order, failing an innocent
campaign. So the replay is service-wide and ordered:
1. **When it starts.** The replay job (its own task and its own login role, `df_finance_replay`,
   17.6) reads O1's **lock-free service state and recovery epoch** in one read
   (`df_runtime.service_state() -> (serving, recovery_epoch)`; a new request to O1) and starts when
   the epoch is greater than the one recorded in `finance.replay_state` and the serving state is
   **`restoring`** (O1's restore step sets it, instead of `serving`, when it raises the epoch), and
   on operator command. It sets `replay_state.status = 'replaying'` (through `apply_change`, an
   unjournaled change). While the state is `restoring` O1 refuses runtime claims, `place`, `renew`
   and `fence_current`, F's admission answers `PaidWorkFrozen`, live `commit_event` is refused, and
   every `reconstruct_campaign` call answers `Failed(replay_pending)`. `replay_event` runs only in
   `restoring` (17.6).
2. It reads the confirmed journal tail, **all `finance_event` entries of every stream** (payer,
   session, campaign, account, platform), beyond the database's high-water position **per stream**
   (the stream heads), through O2's reader (22.3), in **journal order** (the sequence O2 stamped,
   then position in the batch). It never orders by `global_seq`.
3. It applies each missing event through **`finance.replay_event`** (17.6), not `commit_event`:
   original time and period from the payload, stored hash and chain position verified, the
   equations run, `event_marker` and `restore_replay` (recovery epoch, journal sequence, position)
   written, nothing re-journaled. Each event's `payload_schema` is upgraded to the current one by
   `apply_change` (17.6); an unsupported version stops the replay with an alert. Events at or below
   a stream's head are compared (hash equality) and skipped. A replayed event whose prerequisite
   row is absent from a *confirmed* event is a failure of the replay, an alert and a stop, not a skip.
4. **Duration target and progress alert.** All paid work is blocked until step A finishes, so it
   has a target and a visible progress: `restore.replay_target_seconds = 300` for the journal
   tail since the last backup (proposed; measured in round 6 against a tail of one day), the
   job updates `replay_state.events_applied` and `last_progress_at_ms` at least every
   `restore.replay_progress_seconds = 10`, exports `df_finance_replay_events_remaining` and
   `df_finance_replay_seconds`, and raises a P1 alert when no progress has been made for
   `restore.replay_stall_seconds = 60` or when the target is exceeded. The operator console shows
   the replay on the finance page. A stalled replay never reopens paid work.
5. **Step A.5: rebuild the work items.** `work_item` rows are not journaled (they are re-derived
   state, 17.6), so after a replay nothing would expire. The job rebuilds, in one pass through
   `apply_change` (unjournaled `work_item_upsert` changes, idempotent on (kind, ref)), the rows
   for every non-terminal object: for each attempt not terminal, the `reserved_expiry`,
   `prepared_expiry`, `sent_reconcile`, `customer_hold`, `finalization_wait`, `usage_missing`,
   `usability_deadline` and `unknown_resolution` items its state calls for, due at the times
   computed from its stored `prepared_at_ms`, `sent_at_ms` and `result_at_ms`; the `exposure_write_off`
   item for each unmatched exposure; the `quote_expiry` item for each open or approved quote; the
   `session_finish` item for each `settling` session; and the `settle` item for each attempt whose
   facts are complete and not settled. Items already due are due at once. The period close
   needs no item (it is operator-driven). A test checks the rebuilt set equals the set the live
   flows would have written (26.2).
6. **The independent check, then completion.** When step A.5 ends the replay job sets
   `replay_state.status = 'checking'`. The live service (`df_finance`, a different role and code
   path) then runs the replay check of 22.3 as an in-memory Rust model over the confirmed journal and
   the live rows, and records its result with `record_replay_check(result, stream_heads_digest,
   journal_sequence)`. Only then can the replay job call `replay_complete`, which **refuses unless a
   `pass` row exists whose digest equals the live stream-heads digest at that moment** (17.6); it
   then sets `replay_state.status = 'complete'` and calls O1's `df_runtime.restore_complete(
   recovery_epoch)`, which moves the service from `restoring` to `serving`, in the same transaction.
   If the check fails, is missing or goes stale, or the replay stalls, the service stays
   `restoring`: no runtime claims, nothing is dispatched, an alert fires. Paid admission reopens only
   after that.

**Step B: the per-campaign reconstruction, which only reconciles.** `reconstruct_campaign { tenant,
campaign, fence, reconstruction_id }` on the control plane's internal mTLS listener (authorized by
the placement certificate) is idempotent per `reconstruction_id` (`finance.reconstruction`): a repeat
returns the stored outcome. O1 freezes paid work for a campaign only when money reconstruction is
missing or failed (plan J10), not on every crash. The endpoint makes no provider call and, targeting
under 5 s for a campaign with up to 64 open attempts:
1. **Replay present?** If step A has not completed, answer `Failed(replay_pending)`; F's
   reconciliation retries the campaign when it has.
2. **Dispatching entries.** For every attempt journaled by the dispatcher as `Dispatching` (O1's
   entries) and not `Sent` or later in the database: its admission is back (step A replayed it; the
   admission event precedes the `Dispatching` entry in the journal), and the attempt is moved to
   `OutcomeUnknown` with its worst-case exposure (never resent). The reason is
   `restore_dispatching`.
3. **Conservation.** Check the campaign's subledger equations (21.6, 21.7) over the rebuilt rows.
4. **Result.** In F's final transaction, finance rows first, call
   `df_runtime.record_money_state(tenant, campaign, reconstruction_id, outcome)` with
   `Reconstructed` or `Failed { reason }`. **`clear_paid_work_freeze` does not exist** (O1 10.2
   item 7); `record_money_state` is the only way out of `reconstructing` or `frozen` and is a
   compare-and-set on the `reconstruction_id`, so a stale result is ignored. Any mismatch is a
   `Failed`, an alert, and read-only play and export continue (J10). F's reconciliation retries a
   failed one until it succeeds.

- Purchases lost by a restore are re-posted by processor reconciliation from the processor's state
  (idempotent by payment reference); refunds and disputes likewise.
- A game save branch or restore never touches finance: no finance API takes a game revision, and
  restore keeps attempts and reservations under reconciliation (O1 9.5, SAVE-03, plan 13.7).

### 22.3 The journal reader, the replay check and the fail-closed gate

**What F needs from O2's journal** (requests to O2, restored from round 0; round 0b dropped them in
error): `journal.confirmed_tail(kind, stream_kind, stream_id, campaign_id, after_sequence, limit)`,
the confirmed entries of a kind in journal order, selectable by stream or campaign; the stamped
metadata of 22.1 on each pending row (O2 stamps `stream_kind`, `stream_id` and `campaign_id` from
the submitter's metadata, so F's reads are index lookups, not payload scans);
`journal.oldest_unconfirmed_age(kind)`; and the sequence and in-batch position of every entry.
Replay (22.2), `journal_compare` (17.7) and the fail-closed gate all read through this.

**The replay check.** After step A.5, before paid admission reopens, the `campaign_restore` job
runs the check as an **in-memory model in Rust** (no scratch schema, no DDL, so a read-only role can
run it): it reads the confirmed journal from O2's reader and the live rows with `SELECT`, applies the
typed after-images to an empty model of the lots, allocations, reservations, shares, attempts, usage
events and receipts, and compares the model row for row with the live tables and the stream heads.
Any difference is a `fail` record and blocks reopening. **The comparison excludes the columns
that are never journaled** (22.1): `billing_country`, `billing_region`, `email_grant_key` and
`evidence_state`, which are rebuilt from B afterwards and checked by their own reconciliation item,
and the unjournaled `work_item`, `flow_lock`, `event_marker`, `replay_state`, `restore_replay` and
`replay_check_record` rows, of which `work_item` is checked against the set step A.5 derives. The
result reaches the database only through `record_replay_check`, which `replay_complete` consults
(17.6). The same check runs in tests (26.2: the model equals the original database on every
journaled column, and an in-flight attempt settles after replay exactly once).

**Fail closed.** If `journal.oldest_unconfirmed_age('finance_event')` exceeds
`recovery_journal.maximum_unconfirmed_seconds = 60` (proposed), admission refuses with
`RecoveryJournalBehind` (plan 15.4: failure of durable financial admission blocks new paid work);
settlement, release and refunds continue (they only reduce exposure or are already committed).

## 23. Accounting exports (owner bookkeeping)

Built on demand from immutable data by the export job; downloaded only by an operator with
`FinanceExport`; formats CSV (RFC 4180, UTF-8, header row) and JSON with a schema name. Detail files
carry `amount_micro_dollars` (exact integer) and `amount_usd` (six decimals); summary files carry
cents by the cumulative rule of 4.10 with one rounding line per day to the single rounding account.

### 23.1 What marks a file

Every export file begins with a header block (JSON: top-level fields; CSV: a leading `#` metadata
section plus a sidecar manifest): `period_id`, the accounting time zone and cut-off rule
(**posting date**, 17.8), `book` (`general`, or `promo` / `exposure`), `ledger = true | false`, the
as-of watermark (`as_of_ms`, the confirmed journal sequence, and the digest of the stream heads, 17.5), a
`provisional` flag, the generation number, and the SHA-256 digest of the file bytes, which is also
recorded in `finance.export_manifest`. **The Promo and Exposure books are memoranda and are marked
`ledger = false`**; they are exported in separate files named `memo-...` and are never summed into
the trial balance.

### 23.2 The files

| Export | Content |
|---|---|
| Chart of accounts (`chart-of-accounts`) | for every code: code, name, book, **type** (asset, liability, equity, revenue, contra-revenue, expense, memo), **normal side**, enforced flag, dimensions, whether it is in the ledger |
| Trial balance (`trial-balance-<period>`) | General book only, per account code and currency: opening (from the prior `period_balance`), debits, credits, closing; compared in a test with an independent recomputation from the lines (it sums to zero per currency) |
| General ledger detail (`journal-<period>`) | every entry and line: entry id, kind, posting date, effective date, account code and name, dimensions (ids only), debit, credit, reason, case reference, external reference kind and value, correlation ids, hash |
| Summary journal (`summary-journal-<period>`) | one balanced journal per day summarized by account code in cents (4.10), a single rounding line (5700) per day, in the column shape bookkeeping tools import (date, journal number, account, debit, credit, memo) |
| Adjustment listing (`adjustments-<period>`) | every `manual_adjustment`, `manual_refund_payout`, `goodwill_grant`, `reversal` and `customer_credit` with actor, approver, case, reason code and amount, and for a reversal the entry it reverses |
| Deferred revenue roll-forward (`deferred-revenue-<period>`) | purchased credit liability (2110+2120+2130+2140): opening + purchases (net of tax) + customer credits (reversals of charges) + refunds returned to credit - consumption recognized (AI usage and table time) - transfers to refund payable - external refunds - dispute freezes lost (extinguished at the loss) - chargeback recoveries (liability released into a charged-back lot) - closure rounding = closing, closing equal to the ledger and to the `period_balance` snapshot (test: it closes after every kind of customer credit, refund return and chargeback recovery) |
| Refund payable roll-forward (`refund-payable-<period>`) | opening + requested - paid - manual payouts - returned to credit - extinguished by lost disputes = closing |
| Promotional credit (`memo-promotional-<period>`) | Promo book, `ledger = false`: granted by source, consumed at retail, expired, forfeited at closure, outstanding; supplier cost of promotional usage (5110, a General-book figure shown beside); in gross mode also 5300/4200 |
| Company loss (`company-loss-<period>`) | 5200 by class and provider; forgone revenue (`meter_unfunded`, `refunded_externally`) as a separate memo table |
| Supplier liability (`supplier-<period>`) | payable and accrued by provider, open exposure by age (`memo-`), invoice matches and tolerance rounding |
| Processor (`processor-<period>`) | charges, fees, refunds, disputes, payouts, clearing balance |
| Bank tie-out (`bank-tieout-<period>`) | payouts, manual refund payouts and supplier payments against the imported bank statement lines: matched, unmatched entries, unmatched lines, and the difference to the `bank_operating` balance |
| Tax (`tax-<period>`) | per jurisdiction collected and reversed (empty while `tax.mode = none`) |
| Breakage schedule (`breakage-<period>`) | only when `breakage.policy` is not `none`: an estimate for the accountant, never a posting that reduces a customer's balance |
| Treasury (`liquidity-<as-of>`) | the 6.1 liquidity view |
| Personal usage statement (`dungeonflux.finance.export.usage-statement/1`) | per account, for O2's export source (12.6) |

### 23.3 Closing a month, as a bookkeeper does it

1. Run the full reconciliation for the period (21); an operator reviews each open difference and the
   bank tie-out; a second person signs the run off (`reconciliation_signoff`).
2. `close_period` (17.8) refuses until the sign-off, the confirmed journal, and the earlier
   periods are in place, then writes the `period_balance` snapshot and the digest.
3. The bookkeeper takes the chart, the closed trial balance, the summary journal and the
   deferred-revenue roll-forward, whose headers carry the watermark and digest, and checks them
   against the manifest. Late items (a fee, a payout, a supplier invoice) arriving after the close
   post to the next open period by the posting-date rule, and appear in that month's adjustment
   listing.

### 23.4 Rounding in the files

Cents by the cumulative rule (4.10): per account, per day, `R(cumulative) - R(previous cumulative)`.
The days of a month sum to the monthly cents figure of the same rule. The one rounding account (5700)
absorbs the difference needed to balance each day in cents, and the sum of its daily lines is the
month's rounding. A per-account rounding line is not used.

### 23.5 Regeneration

A closed period regenerates **byte for byte** from the snapshot and the immutable lines (test). An
open period's files carry the watermark and `provisional = true` and are not expected to match a
later run. The tests for the files are in 26.4.

Revenue recognition as built (owner and accountant to confirm, 28.1): purchased credit is deferred
revenue until consumed; consumption is recognized at settlement (AI usage) and at segment pricing
(table time); holds are not revenue; promotional consumption is not revenue in `net` mode (the
default, ruling (a)). Breakage default `none`: no breakage revenue is ever recognized
operationally; customer claims stay whole.

## 24. Observability and dev CLI

- Audit lines through `df_observe::record_audit_line`, one per event, ids and closed codes only:
  `FINANCE <posted|admitted|refused|prepared|prepared_cancelled|sent|unsent|outcome|usage|settled|
  released|credited|company_loss|meter_hold|meter_priced|meter_unfunded|session_settling|
  session_closed|trial_granted|trial_refused|purchase_applied|allocated|withdrawn|epoch|policy|
  quote|quote_approved|quote_used|refund_requested|refund_paid|refund_manual|refund_review|
  refund_unknown|dispute|standing|reconciled|difference|reconstructed|replayed|outcome_event|
  period_closed|integrity|forfeited|export>
  entry= kind= session= attempt= reservation= lot= payer=<8 hex of hash> amount_micros=
  reason= state= ms=`. Never prompts, emails, card data or private text.
- Metrics through O1's registry (D20): `df_finance_admission_ms`, `df_finance_admissions_total
  {outcome}`, `df_finance_open_holds_micro_dollars`, `df_finance_oldest_hold_seconds`,
  `df_finance_unknown_attempts`, `df_finance_customer_liability_micro_dollars{class}`,
  `df_finance_supplier_exposure_micro_dollars`, `df_finance_company_loss_micro_dollars_total
  {class}`, `df_finance_forgone_revenue_micro_dollars_total{reason}`,
  `df_finance_reconciliation_differences`, `df_finance_journal_unconfirmed_seconds`,
  `df_finance_funding_feed_lag_seconds` (a measurement, not a gate),
  `df_finance_duplicate_effects_total`, `df_finance_retry_supplier_micro_dollars_total`,
  `df_finance_charges_micro_dollars_total{price_version}`,
  `df_finance_cost_book_disagreement_micro_dollars_total`, `df_finance_lock_rank_violations_total`,
  `df_finance_content_screens_total{result}`, `df_finance_integrity_failures_total{control}`,
  `df_finance_work_items_overdue{kind}`. Alerts (plan 7.3): rising retry cost, unexplained hold age,
  duplicate settlement attempts, low provider prepaid credit
  (`supplier.low_prepaid_alert_micro_dollars`, an alert, not a cap), any reconciliation difference
  outside tolerance, any integrity failure, a settling session past its alert age.
- `dev finance` (read only): five balances, journal tail, holds and attempts by state and age,
  meter per session, feed lag, journal backlog, last reconciliation and integrity run, stream chain
  heads; `dev finance entry <id>`; `dev finance check` runs the invariant checks of 21.1, 21.6 and
  21.7 and prints the result. The only dev mutation is `dev finance reconcile` (writes a run).

## 25. Configuration (`content/finance.json`, schema `dungeonflux.finance/1`)

Validated at load; a missing, malformed or out-of-range knob falls back to its default with one
audit line (coding style 8.9). Prices and presets live in the price schedule (9.1).

| Knob | Default | Source |
|---|---|---|
| `currency` | `USD` | plan 6.4 |
| `accounting.time_zone` | `UTC` | ruling (c); changes only from a future period (17.8) |
| `period.precreate_months_ahead` | 12 | 17.8 |
| `trial.enabled`, `trial.amount_micro_dollars` | `true`, `10_000_000` | D3, ruling (b) |
| `trial.expiry_days` | none | D3 |
| `promotional.expiry_days` | none | D3 |
| `promotional.presentation` | `net` | ruling (a) |
| `breakage.policy` | `none` | brief |
| `purchase.presets_micro_dollars`, `purchase.default` | 10, 20, 50 dollars; 20 | plan 5.2 (B renders) |
| `risk.maximum_purchase`, `risk.maximum_outstanding_purchased`, `risk.maximum_daily_new_payer` | $100, $200, $200 | plan 5.2 (proposed) |
| `quote.lifetime_seconds` | 600 (proposed) | plan 9.10 |
| `session.default_per_request_threshold_micro_dollars` | 500_000 (proposed) | plan 4.2 |
| `presets.custom_minimum`, `presets.custom_maximum` | $1/h, $100/h (proposed) | plan 4.3 |
| `pacing.initial_burst_permille` | 250 (proposed) | plan 4.2 |
| `pacing.essential_reserve_micro_dollars` | 100_000 (proposed) | plan 4.2 |
| `funds.low_runway_minutes` | 10 (proposed) | plan 4.1, 9.10 |
| `split.on_exhausted_payer` | `water_fill` (proposed) | plan 5.3 |
| `meter.interval_seconds`, `meter.intervals_ahead` | 60, 2 | plan 4.2, O1 5.2 |
| `meter.settling_poll_seconds`, `session.settling_alert_hours` | 30, 6 | 13.4 |
| `reserved.expiry_seconds`, `reserved.superseded_expiry_seconds` | 300, 60 (proposed) | 12.3 |
| `dispatch.prepared_expiry_seconds`, `dispatch.sent_reconcile_after_seconds` | 120, 300 (proposed) | 12.3, O1 10.2 |
| `hold.unresolved_release_minutes`, `hold.release_minutes.<route>` | 15; unset (video proposed 45) | plan 6.3 |
| `hold.finalization_wait_minutes` | 60 (proposed) | 12.1 |
| `unknown.resolution_hours` | 24 (proposed), from `sent_at` | D25 |
| `usage.report_deadline_seconds` | 300 (proposed) | 12.3 |
| `usability_report_deadline_seconds` | 300 (proposed), non-media only | D21 |
| `media.finalization_alert_hours` | 48 (proposed) | 12.1 |
| `outbox.rescan_window_minutes` | 15 (greater than the transaction timeout, validated) | 12.6 |
| `supplier.exposure_write_off_days` | 45 (proposed) | plan 6.3, 7.3 |
| `supplier.billing.<provider>` | `postpaid` | plan 6.5 |
| `supplier.low_prepaid_alert_micro_dollars.<provider>` | unset (no alert) | plan 7.3 |
| `supplier.invoice_line_tolerance_micro_dollars`, `supplier.invoice_total_tolerance_bps` | 10_000, 50 (proposed) | 4.9 |
| `company_limits.<scope>` | `unlimited` for every scope | LAUNCH hard rules, plan 7.1 |
| `company_concurrency.<scope>` | `unlimited` | LAUNCH hard rules |
| `unpriced_route_policy`, `unpriced_route_supplier_maximum.<route>` | `absorb`, per route | 15.2 |
| `tariff.long_context_policy` | `refuse` above the highest stored tier | 9.1 |
| `accepted_job_policy` | `charge_if_delivered` (proposed) | plan 6.3 |
| `refund.unused_credit_window_days` | none (proposed) | plan 6.7 |
| `refund.processor_refund_window_days` | 180 (proposed); older lots are manual payouts | plan 6.7, ruling (g) |
| `statement.show_quantity` | `true` | ruling (f) |
| `tax.mode`, `tax.jurisdictions` | `none`, `[]` | plan 6.7, owner |
| `manual_adjustment.second_approver_above_micro_dollars` | off (proposed) | plan 7.3 |
| `reconciliation.daily_at_utc`, `integrity.daily_at_utc` | `03:00`, `03:30` | plan 7.3 |
| `close.reviewer_required` | `true` | 17.8 |
| `recovery_journal.maximum_unconfirmed_seconds` | 60 (proposed) | plan 15.4 |
| `restore.campaign_reconciliation_target_ms` | 5000 | coordinator contract (2) |
| `restore.replay_target_seconds`, `restore.replay_progress_seconds`, `restore.replay_stall_seconds` | 300, 10, 60 (proposed) | 22.2 step A |
| `lock_timeout_ms` | 2000 | 19.1 |
| `receipts.retention_days` | 90 | B's pattern |

## 26. Test plan

All tests are sentence-named, use the fake clock, seeded randomness, faithful fakes (B's fake
processor, a fake O1 fence and segment source, a fake O2 outbox, C's fixture cost records) and
never sleep, spend or touch the network. PostgreSQL tests run on the coordinator's scratch server
through the existing env-gated harness, one fresh schema per test, and a round is not reported done
with them skipped. Race tests use controlled interleaving (a second connection runs the competing
statement; the test asserts through `pg_locks` that it waits, then releases the first). Each
behaviour test is written failing first.

Three rules from the round-0 critic apply to every table below:

1. **Real production paths.** A test of a rule calls the production function, never a test-local
   copy. The POT-06 tests call `df_commerce::pot::split_exactly`; a test-local `largest_remainder`
   (S4's `pot.rs` has one) would pass with no F code. S4 is asked to repoint its test to the
   production function (29.4). The admission, settlement and posting tests call the service and the
   definer functions, not helper reimplementations.
2. **A positive test beside every negative one.** "Refuses X" is paired with "admits the same request
   without the defect", built by changing exactly one fact, so a stub that always refuses fails.
3. **Each invariant has a mutant.** The PostgreSQL harness can apply a named mutation to the
   scratch schema (drop a trigger, drop a CHECK, grant a privilege, change the allowlist seed). For
   each invariant test the table names the mutant under which that test **must fail** (26.5); the
   mutation suite fails if the test still passes. Vacuous tests (satisfied by another constraint
   alone, or by an empty grant set) are replaced as listed in 26.2.

### 26.1 Acceptance ids

| Id | Tests (crate) |
|---|---|
| PAY-01 | `a_purchase_posts_only_from_an_authoritative_captured_processor_state` (df-finance); `a_purchase_entry_without_a_processor_payment_state_row_is_refused` (PG); `no_finance_call_exists_on_the_checkout_return_path` (df-portal, with B) |
| PAY-02 | `the_same_payment_applied_five_times_under_new_event_ids_posts_one_purchase`; `five_concurrent_applies_of_old_and_new_states_for_one_payment_post_once_and_none_errors` (PG, five connections, controlled interleaving; the last one wins the row lock and finds `AlreadyCurrent`) |
| PAY-03 | `a_refunded_state_seen_before_the_captured_state_never_makes_credit_spendable`; `an_older_processor_state_cannot_lower_the_refunded_total_and_returns_already_current` |
| PAY-04 | `a_payment_confirmed_after_the_browser_closed_shows_one_lot_on_the_status_read` |
| PAY-05 | `two_concurrent_purchases_and_allocations_keep_their_own_lots_and_lose_no_update` (PG, interleaved) |
| PAY-06 | `the_operator_breakdown_for_one_account_lists_none_of_another_accounts_lots_or_allocations` (B owns the card-fingerprint view) |
| POT-01 | `six_concurrent_admissions_against_a_small_pot_never_overdraw_or_reserve_more_than_it_holds` (PG, six connections); `admission_refuses_once_the_pot_cannot_hold_the_maximum` (pure) |
| POT-02 | `withdrawing_while_a_job_is_admitted_releases_only_the_unreserved_part`; `a_release_into_a_withdrawn_allocation_lands_in_available_not_allocated`; `the_last_hold_of_a_withdrawn_allocation_sweeps_its_allocated_to_available` |
| POT-03 | `a_policy_raised_above_a_payers_consent_excludes_that_payer_from_new_work_until_they_consent`; `a_lowered_ceiling_refuses_new_work_and_keeps_sent_work` |
| POT-04 | `a_removed_contributors_in_flight_hold_settles_against_their_original_share_and_new_work_skips_them` (with S1's fake feed); `f_down_across_three_events_applies_each_once` (restart, `campaign_heads_after`, `funding_effect` unique); `a_closing_account_is_excluded_from_the_shares_at_admission`; `a_regressed_event_seq_or_changed_feed_epoch_rederives_allocation_status_and_alerts`; `f_creates_its_position_row_in_the_transaction_of_its_first_grant`; `a_sweep_never_overwrites_a_position_written_by_a_concurrent_first_grant` (interleaving: the grant takes its snapshot, a `left` commits, the sweep runs `ON CONFLICT DO NOTHING`, the grant commits; the `left` is applied once, not skipped) |
| POT-05 | `an_epoch_change_mid_session_applies_to_later_reservations_only` |
| POT-06 | `the_plans_three_contributor_example_splits_eight_dollars_four_two_two` and `a_thousand_one_micro_charges_across_three_equal_payers_lose_nothing` and `largest_remainder_splits_sum_exactly_and_never_raise_the_table_total` (property), **each calling `df_commerce::pot::split_exactly`** |
| JOB-01 | `an_unknown_outcome_keeps_full_supplier_exposure_and_a_second_admission_for_the_attempt_is_refused`; `the_customer_hold_is_released_after_fifteen_minutes_counted_from_sent_at_while_supplier_exposure_stays_open` |
| JOB-02 | `a_settlement_for_a_superseded_attempt_charges_the_customer_nothing`; `a_second_settlement_for_one_attempt_changes_nothing` |
| JOB-03 | `every_admission_refusal_is_provoked_by_one_change_to_an_admissible_request_and_holds_nothing_and_writes_nothing` (table-driven, with `the_unchanged_request_is_admitted`); the circuit breaker is O1's |
| JOB-04 | `invalid_output_is_not_charged_and_its_supplier_cost_is_company_loss`; `output_lost_after_provider_success_is_not_charged`; `duplicate_output_is_charged_once`; `a_technical_failure_credit_returns_the_charge_once_to_the_original_lots`; `two_concurrent_credits_with_different_operation_ids_never_credit_more_than_the_charge` (PG, interleaved, mutant: remove the row lock); `a_credit_splits_across_payer_shares_by_remaining_reversible_amount`; `a_technical_failure_credit_reclassifies_5100_and_5110_to_5200_cumulatively_and_exactly` |
| JOB-05 | `a_late_result_after_the_customer_hold_was_released_is_company_funded` |
| JOB-06 (new) | `a_reserved_attempt_with_no_claim_is_cancelled_at_its_expiry_and_releases_hold_exposure_and_the_media_quota`; `a_reserved_attempt_with_a_superseded_fence_is_cancelled`; `a_dispatch_prepared_attempt_past_its_expiry_becomes_cancelled_unsent_and_the_dispatcher_is_refused`; `mark_attempt_sent_racing_the_prepared_expiry_has_exactly_one_winner` (PG, interleaved); `a_company_admission_is_expired_like_a_customer_one` |
| JOB-07 (new) | `a_media_result_awaiting_finalization_is_never_booked_as_usability_unreported`; `a_non_media_result_with_no_usability_report_300_seconds_after_the_result_is_company_loss`; `a_result_awaiting_finalization_releases_the_hold_after_the_finalization_wait_and_a_later_ready_is_company_funded`; `the_hold_clock_starts_at_sent_at_not_at_admission` |
| JOB-08 (new) | `a_quote_funds_one_admission_and_the_second_is_refused_quote_already_used`; `a_refused_admission_leaves_its_quote_approved`; `two_concurrent_admissions_with_one_quote_admit_one` (PG) |
| JOB-09 (new) | `an_admission_for_a_category_with_no_tariff_row_is_refused_category_unpriced_not_priced_at_zero`; `maximum_units_missing_a_declared_category_is_refused`; `a_request_above_the_highest_stored_context_tier_is_refused`; `a_usage_report_with_dollars_and_no_units_is_rejected`; `a_usage_derived_from_a_hard_coded_or_float_price_is_estimated_and_priced_from_the_pinned_row`; `every_priced_route_category_has_a_tariff_row` (over C's `ROUTES` and the price schedule); `a_pending_route_is_refused_in_hosted_paid_mode`; `jev_is_refused_until_it_is_metered` |
| JOB-10 (new) | `a_content_screen_runs_admit_prepare_send_outcome_settle_and_each_expiry_without_a_campaign` (account principal, `account` stream, null fence, `current_dispatcher()` still required); `a_null_fence_on_a_campaign_attempt_is_refused`; `screens_of_two_accounts_do_not_contend_on_one_stream_head`; `an_account_principal_with_a_table_payer_is_refused` |
| JOB-11 (new) | `a_result_known_attempt_with_no_usage_from_either_source_becomes_unknown_after_the_deadline_with_the_hold_released_and_the_exposure_kept`; `a_dispatcher_supplied_usage_unit_report_settles_without_a_runtime_report`; `session_finish_completes_after_a_usage_missing_attempt_is_released`; `a_runtime_that_dies_between_the_dispatcher_result_and_record_usage_strands_nothing` |
| TIME-01 | `paused_and_all_away_intervals_are_never_priced` (O1 segment fixtures) |
| TIME-02 | `a_credited_blocking_interval_is_not_charged` |
| TIME-03 | `pricing_a_segment_twice_charges_once`; `the_cumulative_meter_never_charges_more_than_the_exact_amount_for_any_split_of_time` (property); `the_price_is_over_opened_at_to_closed_at_and_the_open_segment_is_never_priced`; `a_negative_interval_is_refused_with_an_alert` |
| TIME-04 (new) | `a_charge_larger_than_the_remaining_holds_is_capped_and_the_shortfall_is_forgone_revenue_meter_unfunded`; `a_capped_interval_is_not_charged_again_when_the_pot_is_refilled`; `session_end_keeps_the_meter_holds_until_every_closed_segment_is_priced`; `end_session_funding_called_by_the_lifecycle_and_by_the_sweeper_for_an_abandoned_end_is_idempotent`; `a_lost_end_call_is_healed_by_the_meter_workers_poll`; `the_finish_transaction_calls_settlement_finished_once_and_ended_follows`; `a_session_end_with_an_unpriced_segment_does_not_finish` |
| REF-01 | `a_partial_refund_across_lots_takes_newest_lots_first_with_their_processor_references_and_tax` |
| REF-02 | `a_refund_whose_processor_call_times_out_stays_payable_and_never_returns_to_credit_by_itself`; `a_repeated_refund_request_makes_one_processor_call`; `a_failed_refund_goes_to_review_not_back_to_cash_and_credit`; `a_lot_past_the_processor_window_becomes_a_manual_payout_and_posts_through_manual_refund_payout` |
| REF-03 | `one_contributors_lost_dispute_never_debits_another_contributor`; `a_release_into_a_charged_back_lot_is_a_recovery_not_credit`; `a_release_into_a_frozen_lot_posts_to_purchased_frozen_and_returns_to_available_when_the_dispute_is_won`; `a_pending_refund_payable_is_blocked_at_dispute_open_and_extinguished_at_the_loss`; `the_reserved_portion_at_dispute_open_stays_reserved_and_resolves_as_revenue_or_recovery`; `a_credit_to_a_charged_back_lot_is_refused`; `an_external_refund_drains_available_then_allocated_then_reserved_then_consumed`; `a_dispute_lost_for_less_than_the_frozen_amount_extinguishes_only_that_part_and_releases_the_rest_to_available` (D < frozen, the partial case, lot stays Active); `a_dispute_lost_for_more_than_frozen_books_only_the_positive_remainder_as_loss`; `a_refund_lot_in_state_unknown_is_not_extinguished_by_a_lost_dispute_and_a_later_processor_payment_still_has_its_liability`; `only_pending_failed_review_manual_pending_and_blocked_by_dispute_refund_lots_are_extinguished` (each of the eight refund_lot states is provoked in turn; `unknown`, `paid`, `returned_to_credit` and `cancelled_dispute_lost` are not); `a_dispute_lost_entry_with_the_release_line_is_accepted_by_the_allowlist`; `the_loss_is_never_negative_for_any_d_frozen_and_pending_refund` (property); `a_pending_refund_the_remainder_cannot_cover_whole_stays_payable`; `the_lot_state_after_a_lost_dispute_follows_the_balances_charged_back_only_when_nothing_backs_the_lot` |
| OPS-02 | `a_restore_missing_recent_finance_entries_keeps_paid_work_frozen_until_the_journal_tail_is_replayed`; `per_campaign_reconstruction_finishes_within_its_target_and_ends_in_record_money_state`; `a_journaled_dispatch_missing_from_the_database_returns_as_unknown_exposure_never_resent`; `a_replayed_database_equals_the_original_row_for_row_in_lots_allocations_reservations_attempts_usage_events_and_receipts`; `an_in_flight_attempt_settles_exactly_once_after_replay`; `a_replayed_event_with_a_missing_prerequisite_fails_the_reconstruction`; `a_repeated_reconstruction_id_returns_the_stored_outcome`; `replay_applies_events_through_replay_event_and_never_re_journals_them`; `replayed_entry_hashes_and_chain_positions_equal_the_originals` (hash-match test); `a_fixture_journal_of_version_1_entries_replays_and_compares_under_a_build_whose_current_hash_version_is_2` (a fixture across a hash-version change; `replay_event`, `journal_compare` and the integrity job use the stored version); `a_replay_payload_with_one_changed_amount_fails_the_hash_check`; `replay_event_is_refused_unless_the_service_is_restoring_and_commit_event_is_refused_during_a_replay`; `a_restore_leaves_the_service_restoring_so_runtime_claims_fence_current_and_admission_refuse_until_replay_complete_moves_it_to_serving`; `a_failed_or_stalled_replay_leaves_the_service_restoring`; `the_live_df_finance_connection_cannot_execute_replay_event_or_set_role_to_the_replay_role`; `the_post_replay_check_runs_as_a_different_role_and_code_path_and_catches_a_bad_replay_event`; `the_replay_never_reads_the_clock_original_times_and_periods_survive_a_database_clock_a_year_ahead`; `a_fixture_journal_written_by_the_previous_payload_schema_replays_under_the_current_code_and_rebuilds_the_same_rows`; `an_unsupported_payload_schema_stops_the_replay_with_an_alert`; `step_a5_rebuilds_the_work_items_so_that_a_restored_reserved_attempt_expires_and_a_restored_sent_attempt_reconciles`; `the_replay_check_ignores_the_never_journaled_columns_and_checks_the_rebuild_of_them_separately`; `a_stalled_replay_raises_a_progress_alert_and_never_reopens_paid_work`; `the_replay_starts_when_the_lock_free_epoch_read_shows_a_new_epoch_and_the_service_not_serving`; `the_full_ordered_replay_covers_every_stream_before_any_campaign_is_reconstructed`; `a_campaign_reconstruction_before_the_replay_completes_answers_replay_pending`; `a_payer_stream_event_spanning_two_campaigns_replays_once_and_keeps_chain_order`; `replay_orders_by_journal_sequence_and_never_by_global_seq` (a lower global_seq committing late); `a_price_version_published_before_the_restore_prices_an_in_flight_session_after_replay`; `a_trial_grant_survives_replay_and_blocks_new_grants_until_its_key_is_rebuilt`; `billing_evidence_is_rebuilt_from_b_after_replay` |
| OPS-04 | `a_full_reconciliation_of_a_seeded_month_reports_nothing_outside_tolerance`; and for each reconciliation kind `a_planted_<kind>_fault_is_reported_and_not_balanced_away` (`journal_integrity`, `journal_compare`, `integrity_controls`, `processor`, `payouts`, `bank`, `supplier_usage`, `reservations`, `meter`, `subledger`, `outcome_feeds`, `campaign_restore`); `invoice_spend_no_attempt_explains_is_unmetered_route_with_an_alert`; `a_difference_inside_the_supplier_tolerance_is_within_tolerance_and_posts_to_rounding` |
| SAVE-02 (money half) | `a_retried_admission_with_the_same_operation_returns_the_original_reservation`; `a_retry_after_a_refusal_is_evaluated_afresh_and_replays_no_stored_refusal` (top up between the two calls); `the_fingerprint_includes_the_attempt_id` |
| SAVE-03 (money half) | `no_finance_operation_accepts_a_game_revision_and_a_branch_creates_no_funding` |
| UX-02 (money half) | `a_pending_purchase_never_appears_in_any_spendable_balance` |
| UX-04 (money half) | `at_zero_available_the_session_reads_funding_required_and_admission_refuses_new_paid_work` |
| AUTH-02 (money half) | `a_display_principal_is_refused_by_every_finance_command` |
| OPS-01 (money half) | `settlement_and_release_complete_while_no_owner_holds_the_campaign` |

S4's skeleton names for the system halves (acceptance matrix) are kept as thin wrappers over these
when S4 wires its crate to `df-finance` (asked of S4 through the coordinator).

### 26.2 Ledger and database invariants (PostgreSQL, `df-persistence`)

| Test | Paired mutant (must make it fail) |
|---|---|
| `an_unbalanced_entry_cannot_commit` | drop the balance trigger |
| `an_entry_whose_header_line_count_disagrees_with_its_lines_fails_at_commit` and `a_one_line_entry_with_a_two_line_header_fails_at_commit` (replaces the vacuous one-line test: the `line_count >= 2` CHECK alone must not satisfy it, so the test builds a header that passes the CHECK) | drop the count trigger |
| `a_line_added_to_an_earlier_entry_fails_at_commit` | drop the count trigger |
| `a_journal_line_cannot_be_updated_or_deleted_or_truncated` | drop the append-only and TRUNCATE triggers; grant `TRUNCATE` |
| `a_semantic_mint_is_refused`: Dr an expense / Cr `purchased_available` is refused by `commit_event` even though it balances; so is every (code, side) not in the kind's rows, kind by kind | add the pair to the seed |
| `a_manual_adjustment_over_a_customer_credit_account_is_refused`: 1xxx, 21xx, 2500, 22xx, 4200, 5300 and the Exposure book | add the code to the manual rows |
| `a_promotional_lot_under_a_purchased_code_cannot_exist` and `a_key_for_a_lot_of_another_payer_or_currency_cannot_exist` | drop the key trigger |
| `a_line_crossing_from_the_promo_book_to_the_exposure_book_is_refused` (Dr `promotional_offset` / Cr `supplier_exposure_open`) | merge the books in the balance check |
| `approver_equal_to_actor_is_refused` (the CHECK and the function) | drop the CHECK and the function test |
| `a_purchase_without_a_payment_state_row_and_a_trial_grant_without_a_trial_row_are_refused` | drop the anchor check |
| `a_line_that_would_make_customer_credit_negative_is_refused` | drop the balance CHECK |
| `the_role_bypass_attempts_fail`: as `df_finance` and the runtime roles: `SET session_replication_role = replica`, `ALTER TABLE ... DISABLE TRIGGER`, `TRUNCATE`, `INSERT` into a journal table, `SET CONSTRAINTS ALL IMMEDIATE` then `commit_event` (still commits correctly, the function resets the mode) | grant the privilege |
| `the_exact_execute_grant_list_matches_the_manifest_and_no_role_can_write_any_finance_table` (replaces the empty-set test: it asserts each role holds exactly the listed functions, `PUBLIC` holds none, and `has_table_privilege(.., 'INSERT,UPDATE,DELETE,TRUNCATE')` is false for every finance table) | grant one table |
| `the_integrity_controls_job_detects_a_disabled_trigger_a_changed_function_body_and_a_changed_acl` | none; this is the detection test |
| `the_hash_chain_detects_an_edited_deleted_or_inserted_entry` (edit as the owner) and `the_daily_journal_compare_reports_a_database_that_differs_from_the_sealed_journal` | drop the chain check |
| `an_entry_in_a_closed_period_is_refused`; `a_closed_period_cannot_be_reopened_or_deleted`; `the_period_id_is_derived_from_the_database_clock_and_a_caller_value_is_ignored`; `the_first_entry_of_a_new_month_finds_its_precreated_period`; `a_month_rollover_in_the_configured_time_zone_assigns_the_posting_date_period`; `close_period_refuses_without_a_clean_reconciliation_a_signoff_by_a_second_person_or_a_confirmed_journal`; `close_period_writes_an_immutable_snapshot_and_waits_for_in_flight_postings` | drop the period trigger; drop the exclusive lock |
| `a_reversal_must_negate_the_original_exactly_and_only_once`; `a_reversal_of_a_customer_affecting_kind_is_refused` | drop the unique constraint and the negation trigger |
| `the_reserved_key_always_equals_the_sum_of_open_shares`; `allocation_totals_equal_their_keys`; `a_lot_cannot_close_with_a_balance_or_an_open_share`; `lot_conservation_holds_after_every_flow`; `an_allocation_cannot_close_with_allocated_or_reserved_credit` | drop each subledger trigger in turn |
| `the_trial_balance_agrees_with_an_independent_recomputation_from_the_lines_and_the_snapshots` (replaces the "sums to zero" test, which follows from the balance trigger alone) | change one line of the export builder |
| `the_whole_flow_set_run_in_parallel_raises_no_deadlock`: admission, settlement, user release, O2's `cancel_unclaimed_effect` racing a user cancel, the 24 h resolution, meter-hold renewal, allocate, withdraw, refund request, dispute open, credit, `mark_attempt_sent`, `apply_processor_state`, lot closing, all against shared lots and sessions, randomized interleavings, 1,000 rounds: **no `40P01`, no `40001`, no lock-rank violation** | take the balance rows after the fence step |
| `a_flow_that_locks_out_of_rank_order_trips_the_rank_assertion` | none |
| `key_creation_is_the_only_place_balance_rows_appear` | allow lazy key creation in `post_entry` |
| `two_sessions_sharing_a_payers_lot_settle_concurrently_without_deadlock_or_lost_totals` (the `shared_lot` test, PG, two connections, controlled interleaving, then the lot's `consumed` equals the sum of both charges and the conservation statement holds) | settlement takes the lot `FOR SHARE` then upgrades |
| `each_conservation_check_is_one_statement_and_sees_one_snapshot` (a concurrent writer cannot make it pass or fail spuriously) | split the check into two statements |
| `a_dispute_lost_for_less_than_the_frozen_amount_books_no_loss_and_releases_the_rest` (D < frozen); `the_dispute_lost_entry_follows_x1_x2_x3_for_every_d` (property, loss never negative) | drop the release line; allow a negative loss |
| `equation_E1_..._E10_each_refuse_a_plan_that_breaks_them` (one test per row of the table in 17.6: revenue off by one micro-dollar, debit to another share's key, a line for another payer's key, a wrong return target, a credit larger than `reversed`, an exposure line that differs from the attempt, a release larger than recorded, a purchase that differs from the payment state, a refund that differs from the refund lot, a negative dispute loss) | remove that equation from `commit_event` |
| `two_after_images_from_the_same_revision_cannot_both_apply` (lost-update race, PG, two connections, controlled interleaving: the second gets `StaleChange`); `a_change_to_a_row_outside_the_flows_lock_set_is_refused` | drop `expected_revision`; skip the lock-set check |
| `every_write_path_to_a_finance_table_is_a_journaled_event_kind` (catalog walk over `pg_proc` source and dependencies; and every flow run once leaves one pending journal row of its declared kind with an `event_marker`) | add a definer that writes a table directly |
| `a_release_into_a_charged_back_lot_takes_the_lot_for_update_and_never_upgrades_a_share_lock` and `release_company_loss_and_chargeback_recovery_racing_a_settlement_on_one_lot_raise_no_deadlock` (PG, interleaved) | take `FOR SHARE` in `release` |
| `replay_event_mutants`: skipping the stored-hash comparison, skipping the serving check, or reading the clock each make `replayed_entry_hashes_and_chain_positions_equal_the_originals`, `replay_event_is_refused_while_the_service_is_serving...` or `the_replay_never_reads_the_clock...` fail | the three mutants named |
| `work_item_replay_state_and_restore_replay_writers_are_only_apply_change_and_the_catalog_walk_set_is_commit_event_replay_event_apply_change_post_entry_lock_flow` (replay_begin, replay_complete and rebuild_work_items write through `apply_change` as unjournaled kinds) | add a definer that writes `work_item`, `replay_state` or `restore_replay` directly (for example `replay_complete` writing `replay_state` itself) |
| `entry_hash_verification_uses_the_stored_version`: `replay_event`, `journal_compare` and the integrity job verify a version-1 entry after the current version is 2; an entry whose canonical bytes gain a field in version 2 still verifies as version 1 | recompute with the current version |
| `replay_complete_refuses_without_a_passing_check_row_for_the_live_stream_heads_digest` (no row; a `fail` row; a `pass` row for an older digest after one more event; the right row succeeds and calls `restore_complete`); `record_replay_check_is_not_executable_by_the_replay_role_and_replay_complete_is_not_executable_by_df_finance`; `the_check_runs_as_an_in_memory_model_with_no_ddl_privilege` | skip the check-row test in `replay_complete`; compare against a stale digest; grant `record_replay_check` to the replay role |
| `the_replay_role_and_the_check_record_write_while_restoring_with_a_stale_checked_at_and_every_other_f_write_is_refused`; `the_replay_role_is_refused_outside_restoring` | apply the reconnect-gate exemption to `commit_event`; drop the `restoring` condition from the exemption |
| `re_attributing_an_entry_to_another_attempt_session_campaign_reservation_or_price_version_changes_its_hash_and_journal_compare_reports_it` (version 1 covers the correlation ids); `an_edited_subledger_row_that_disagrees_with_the_lines_is_caught_by_the_triggers_the_daily_reconciliation_or_the_replay_check` (the controls that cover the not hash-covered after-images, 17.7 item 2) | drop `attempt_id` from the version-1 field list; drop the subledger reconciliation |
| `a_write_without_a_commit_event_fails_at_commit` (the write guard) | drop the guard trigger |
| `two_identical_concurrent_admissions_with_no_session_lock_admit_once_and_the_second_returns_the_stored_outcome` (company and account principals; the unique violation is mapped, never surfaced) | read the receipt before the locks |
| `the_journal_payload_contains_only_allowlisted_fields` (golden list of 22.1; no `billing_region`, no `email_grant_key`, no free text) | add `billing_region` to a lot after-image |
| `the_period_advisory_lock_is_per_period_and_a_close_of_an_old_period_never_stalls_a_current_posting`; `corrects_period_id_is_recorded_when_a_closed_period_is_corrected`; `a_time_zone_change_applies_only_from_a_future_instant` | key the lock globally; drop the history check |
| `journal_compare_and_replay_use_stream_positions_and_a_lower_global_seq_committing_late_is_not_skipped` | use `global_seq` |
| `the_company_exposure_counter_is_locked_after_the_lots_and_before_the_operation_rows` | lock it first |
| `a_forged_payment_state_row_is_caught_by_the_next_processor_reconciliation_and_bounded_by_the_purchase_limits` (the residual of 17.7 is detected, not prevented) | none; this is the detection test |

Pure (`df-commerce`): every posting constructor returns a balanced entry or a typed refusal (one
test per flow of section 16, with the section's numbers as fixtures);
`promotional_credit_cannot_be_moved_into_purchased_credit`;
`trial_credit_is_spent_before_purchased_credit`; `trial_credit_is_never_refundable`;
`a_second_trial_for_the_same_email_key_is_refused`;
`retail_maximums_round_up_and_charges_round_down`; `a_charge_never_exceeds_its_reservation`;
`return_target_answers_every_lot_state_and_allocation_status_pair` (the table of 14.4, exhaustive);
`the_entry_kind_seed_equals_the_pure_allowlist`; `supplier_cost_is_priced_from_units_and_split_by_largest_remainder`;
`the_cents_of_each_day_sum_to_the_cents_of_the_month`; `a_closing_sub_cent_remainder_does_not_block_closure_and_is_rounded_once`;
`promotional_forfeiture_clears_the_promo_book_and_posts_nothing_in_the_general_book`.

### 26.3 Property tests

A deterministic generator (a seeded splitmix64 in the test fixtures module; no new dependency)
drives random sequences of 1,000 operations over 2-6 payers and 1-3 sessions: purchase, trial,
allocate, raise, withdraw, depart, policy change, quote and approve, admit, prepare, send, outcome
(known, unknown, not billed), usability, settle, deadline, credit, meter hold and charge, session
end, refund (success, failure, unknown, manual), dispute (open, won, lost, with holds and refunds
pending), external refund, restore and replay. Each sequence runs against the pure model and the
in-memory `FinanceStore` fake, and the same seeds run against PostgreSQL. After every step:

- `random_finance_sequences_balance_every_entry_in_every_book_and_currency`;
- `random_finance_sequences_never_leave_an_enforced_balance_negative`;
- `random_finance_sequences_conserve_every_lot` (21.7 equations);
- `random_finance_sequences_keep_open_exposure_equal_to_the_open_attempts_worst_cases`;
- `random_finance_sequences_map_every_charge_to_one_attempt_or_one_meter_segment`;
- `random_finance_sequences_map_every_sent_attempt_to_one_exposure_record`;
- `random_finance_sequences_never_charge_a_payer_above_their_authorization`;
- `random_finance_sequences_never_charge_above_a_quote_or_a_ceiling`;
- `random_finance_sequences_leave_no_credit_in_an_allocation_that_has_no_open_hold_and_is_not_active`;
- `random_finance_sequences_never_return_spendable_credit_to_a_frozen_or_charged_back_lot`;
- `random_finance_sequences_satisfy_the_per_kind_equations_in_the_pure_model_and_in_the_database`;
- `random_disputes_with_partial_losses_never_book_a_negative_loss_and_keep_conservation`;
- `random_sequences_with_shared_lots_across_sessions_keep_every_lot_total_exact`;
- `replaying_any_prefix_of_a_sequence_changes_nothing` (idempotency);
- `replaying_the_journal_rebuilds_the_database_exactly`;
- `the_memory_store_and_postgres_agree_on_every_balance_after_the_same_sequence` (contract).

A failing seed is printed and kept as a fixed regression case.

### 26.4 Exports and views

`the_trial_balance_agrees_with_the_period_snapshot`; `the_deferred_revenue_rollforward_closes_to_the_ledger_after_credits_chargeback_recoveries_and_refund_returns`;
`the_refund_payable_rollforward_closes`; `the_summary_journal_balances_in_cents_with_one_rounding_line_per_day`;
`daily_and_monthly_cents_agree`; `a_closed_period_regenerates_byte_for_byte`; `an_open_period_export_carries_its_watermark_and_digest_and_is_marked_provisional`;
`the_promo_and_exposure_books_are_marked_not_ledger_and_never_enter_the_trial_balance`;
`the_chart_of_accounts_export_carries_type_and_normal_side`; `the_bank_tieout_lists_unmatched_payouts_and_unmatched_statement_lines`;
`the_adjustment_listing_shows_actor_approver_and_the_reversed_entry`; `breakage_policy_none_adds_no_breakage_schedule`;
`exports_contain_no_email_name_or_private_text`; `a_late_fee_after_the_close_posts_to_the_next_period_by_posting_date`.

D26 (`df_commerce::views`): `every_customer_view_matches_its_allowlist_golden_schema`;
`distinctive_rates_appear_in_no_key_or_value_of_any_customer_view` (markup 7/4, rate $2.37, odd
tariff digits, provider and model names, supplier cost digits, across every view and fixture);
`the_price_version_label_is_opaque`; `the_customer_module_imports_nothing_from_the_operator_module`;
`quantity_text_is_produced_only_by_the_closed_formatter`; and the three view mutants of 26.5.

### 26.5 Mutants

Every invariant test of 26.2 and every D26 test above is paired with a mutation that the test must
detect: the PostgreSQL mutation suite applies each mutant to a fresh scratch schema, runs the
paired test, and fails the suite if the test passes. Mutants are data (`mutants.toml`: name,
SQL, paired test). The pure tests have source mutants of the same kind: a changed tie-break in
`split_exactly`, a rounding direction flipped, `return_target` answering `Allocated` for a Frozen
lot, the allowlist seed widened, a customer view gaining a `label` field, the markup placed in
`quantity_text`, a provider name in the version label. The mutation suite runs in the nightly lane,
and a new invariant is not accepted without its mutant.

### 26.6 Contract tests with O1 and O2

For O1 (11.4): `mark_attempt_sent_from_a_dispatcher_on_another_worker_is_refused`;
`mark_attempt_sent_with_no_registered_dispatcher_is_refused`;
`mark_attempt_unsent_from_another_instance_is_refused`;
`mark_attempt_unsent_releases_the_hold_and_the_exposure_in_the_same_call`;
`a_sent_attempt_whose_dispatcher_instance_died_becomes_unknown_after_reconciliation_and_no_dispatcher_lookup_exists`;
`a_sent_attempt_older_than_300_seconds_becomes_unknown`; `a_sent_attempt_is_failed_after_24_hours_like_an_unknown_one`;
`record_transport_outcome_from_another_instance_is_refused`; `record_transport_outcome_runs_in_the_callers_transaction`;
`reconstruction_ends_in_record_money_state_and_not_in_a_freeze_clear`;
`the_priced_interval_is_opened_at_to_closed_at_without_proven_until_or_unproven_ms`;
`every_f_transaction_begins_with_enter_service`; `a_halt_maps_to_service_halting_without_waiting`;
`an_f_process_writes_nothing_until_the_failover_check_after_its_reconnect`;
`an_idle_in_transaction_session_is_ended_after_five_seconds`; `the_dispatcher_runtime_and_control_grants_match_the_table_of_17_9`.
For O2 (12.6): `attempt_status_maps_every_f_state_to_the_o2_state_of_11_5`;
`customer_released_is_returned_only_while_the_supplier_outcome_is_unknown`;
`cancel_unclaimed_effect_returns_already_claimed_for_a_prepared_attempt_and_cancelled_when_repeated`;
`the_24_hour_failure_and_the_deadline_cancel_write_one_outcome_event_that_o2_consumes_once`;
`o2_reason_names_map_to_the_loss_classes_of_12_6_and_an_unknown_reason_alerts`;
`a_lower_outbox_id_committing_late_is_still_applied_by_the_rescan`;
`the_personal_usage_statement_contains_only_the_requesters_amounts`;
`a_content_screen_admission_without_a_campaign_or_fence_is_admitted_and_company_loss`;
`an_account_principal_with_a_table_payer_is_refused`.

## 27. Delivery order

| Round | Delivers | Proving tests |
|---|---|---|
| 1 | `df-commerce`: `ledger`, `entry_rules`, `postings` (every flow), `returns`, `funding`, `pot`, `rounding`, `price_schedule`, `tariff`, `quote`, `ceilings`, `meter`, `settlement`, `refund`, `dispute`, `plan`, `company_limits`, `views::customer` and `views::operator`; `spend.rs` `ExposureLimit` with S4's consumer update; property tests on the pure model | 26.2 pure, 26.3 pure, POT-06, TIME-03 property, D26 tests |
| 2 | 0020, 0021; the posting boundary (`lock_flow`, `commit_event`, the allowlist seed, hash chain, periods); `df-persistence::finance_store` (lots, trial, processor state, allocation, withdrawal, policy, epochs, funding feed); `df-finance` service skeleton, ports, fakes, contract tests, tunables, `dev finance`; the mutation harness | 26.2 PG with mutants, PAY-01..05, POT-02..05, trial tests, 17.7 controls |
| 3 | 0022; admission transaction (O1 fence and O2 hook behind fakes until they land), attempts, work items and deadline worker, settlement consumers (runtime and O2), outcome events, credits, quotes and approvals | POT-01, JOB-01..09, SAVE-02, AUTH-02, UX-04, the deadlock test, 26.6 |
| 4 | meter holds and pricing against O1's segment contract; session end, `settling`, the `SettlementFinished` handshake | TIME-01..04 |
| 5 | 0023; refunds (processor and manual), disputes with lot-state rules, external refunds, closure, tax fields, B's processor worker integration | REF-01..03, PAY-03 refund half |
| 6 | 0024; reconciliation jobs, integrity jobs, period close, journal events with after-images, replay and per-campaign reconstruction, exports, bank tie-out, operator views | OPS-02, OPS-04, OPS-01 money half, 26.4 |

## 28. Open owner questions (each is configuration where possible)

Decided by the coordinator on 2026-10-25 and no longer open: promotional presentation (net by
default), one trial per email and per account, accounting time zone (UTC) and cut-off (posting
date), customer-caused screen rejections as company loss with an upload rate limit, refusal of
Pending routes in hosted paid mode, quantity plus charge on statements, promotional forfeiture at
closure and manual payout of old lots.

1. **Revenue recognition** (as built in section 23): purchased credit deferred until consumed;
   AI usage recognized gross at retail at settlement, time at pricing. Confirm with the accountant,
   including whether DungeonFlux is principal for AI usage (gross) as a game-service seller.
2. **Gross promotional presentation**, if the accountant prefers it: `promotional.presentation =
   gross` books promotional revenue and an equal promotional expense (net is the default).
3. **Refund windows**: unused purchased credit refundable with no window (proposed), the processor
   refund age limit (proposed 180 days, then the manual payout), and whether consumed service is
   refunded for documented defects only (plan 6.7 suggestion).
4. **Breakage and unclaimed property**: `breakage.policy = none` (default); whether credit balances
   are subject to unclaimed-property rules in launch jurisdictions (counsel).
5. **Tax jurisdictions and mode**: `tax.mode = none` with launch limited to reviewed jurisdictions;
   at purchase or at consumption once a specialist decides.
6. **Trial abuse controls** beyond one per account and per email (device or card signals) (later).
7. **Company exposure to uncertain jobs** (plan 19): company limits stay unlimited by the standing
   rule; confirm the unknown write-off period (45 days, **booked at the worst case**) and the 24 h
   resolution (D25).
8. **Second approver** for manual adjustments (off for the solo pilot; threshold when staff join).
9. **Split at an exhausted payer**: water-fill (proposed) or refuse.
10. **Accepted-job policy** after a user cancels a sent job (`charge_if_delivered` proposed).
11. **Hold release deadlines**: 15 minutes (video proposed 45), the finalization wait of 60
    minutes, and the low-runway threshold 10 minutes.
12. **Purchase risk limits** ($100, $200, $200/day) and the default per-request threshold ($0.50).
13. **Dispute fees**: expensed as processing fees (as built) and whether a dispute fee is ever
    passed to a customer (proposed never).
14. **Launch effect of ruling (e)**: with nine of fifteen routes Pending, a hosted paid table has no
    microphone transcription, embeddings, figure masks, plate checks, place music, video, splat
    worlds or Jev until C meters them (15.2). Confirm that is the launch scope or order C's work.
15. **Reconciliation tolerance** for supplier invoices ($0.01 per line, 0.5 percent per invoice) and
    the finalization alert age (48 h).

## 29. Coordinator questions, deviations, risks and requests

Settled by D30-D32 and removed from the question list: retiring `credit.rs` and
`CommerceRepository` after S4's system halves pass (D30); admission on the control plane's internal
mTLS listener (D30); `df-finance` as a workspace member with edges `df-persistence -> df-finance`
and `df-portal -> df-finance` and `serde` in `df-commerce` (D30); F owns
`df-types/src/finance_ids.rs` with `LotId`, `AllocationId`, `ReservationId`, `QuoteId`, `EntryId`,
`RefundId`, `PriceVersionId`, and the shared `AttemptId` jointly with O2 (D30); PostgreSQL 17 or
later, target 18 (D30; `UNIQUE NULLS NOT DISTINCT` and `transaction_timeout`); F's funding cursor
in F's own schema and a fresh `member_snapshot` at admission (D31).

1. **Shared tariff and bill parser**: `PriceTable::parse` and `parse_bill` live in `df-tools`
   (`cost.rs`); the price-version snapshot and supplier reconciliation need them in the control
   plane. Ask C to move both into `df_commerce::tariff` (pure) and use them from there (one
   parser).
2. **Migrations** 0020-0024 as above; they follow O1's 0015-0019, so `fence_current` exists.
   `subsystem-architecture.md` rows (file locked by B) go through `request.md` in round 2.
3. Deviation: `spend.rs`'s mandatory platform caps become `ExposureLimit::Unlimited` by default
   (owner rule; plan 7.1's company caps exist as configuration).
4. Deviation: time is held one interval ahead by a control-plane worker rather than inside O1's
   roll transaction, so policy stays in Rust; the cost is that a lagging worker makes time free.
5. Risk: a compromised runtime can misreport usage or usability; under-reporting costs only the
   company (caught by invoice reconciliation); over-reporting is capped by the quote maximum and
   the payer's authorization, and media usability comes from O2's catalog, not the runtime.
6. Risk: the tamper-evidence residual (17.7): a database superuser who also obtains the journal key
   and the Backups credential can rewrite and re-seal history between off-host watermarks.
7. Risk: the definer boundary moves policy checks into SQL; the pure allowlist and the SQL seed are
   generated from one source and compared by a test, and the mutation suite (26.5) is what keeps
   the tests honest.
8. Risk: journal events with row after-images are larger than lines alone; O2's writer batches
   them, and the payload is allowlisted field by field (22.1): opaque ids, processor reference ids
   and a jurisdiction code, no address or other personal field.
9. Risk: the evidence rows that anchor `purchase` and `trial_grant` are written through the same
   service channel as the entries; a forged one is detected and bounded, not prevented (17.7 item 5).
10. Risk: serializing settlements of one payer's sessions on a shared lot (19.1) is deliberate; it
    costs latency only for a payer who funds several sessions at once.

### Requests (also in `artifacts/tmp/agents/F/request.md`)

- **O1, restore handshake (round 0e, ruling (a))**: add a `restoring` value to `df_meta.service_state.serving`;
  the operator restore step sets `restoring` (not `serving`) when it raises the recovery epoch; runtime
  claims, `place`, `renew`, `fence_current` and F's admission refuse while it is `restoring`; a lock-free
  `df_runtime.service_state() -> (serving, recovery_epoch)`; and `df_runtime.restore_complete(recovery_epoch)`,
  EXECUTE to `df_finance_replay` only, called by F's `replay_complete` in the same transaction, moving
  `restoring` to `serving` (17.6, 22.2); and (round 0f) the replay role is exempt from the reconnect gate while
  `restoring`, as O1's failover-check transaction is, so a restored database's old `checked_at` cannot block the
  replay (22.2).
- **O1**: `df_runtime.dispatcher_serves_campaign(tenant, campaign)`, `dispatcher_is_live(instance)`,
  `owner_fence(tenant, campaign)` (lock-free), `table_session_phase(session)` (lock-free; **an absent
  session counts as ended**), `session_meter_closed(session)`, `settlement_finished(tenant,
  session)` (applies `SettlementFinished`, `Settling -> Ended`, EXECUTE to `df_finance`), EXECUTE on
  `current_dispatcher()` for `df_finance_definer`; call `end_session_funding` on `End` (and from the
  sweeper's `abandoned` end); the dispatcher puts provider-returned usage units in
  `record_transport_outcome` (12.1); account-attempt variants (null fence) of send, unsent and
  outcome for content screens (11.1); **no dispatcher lookup of an unknown attempt** (ruling:
  D23 stands; `dispatch_output_exists` and `record_lookup_outcome` are withdrawn); answers in 11.4;
  reconstruction through `reconstruct_campaign` ending in `record_money_state`.
- **O2**: `media.settlement_outcomes_after(after_seq, since_at_ms, limit)` to `df_finance`;
  `at_ms` from the database clock inside the terminal transaction; `journal.submit_pending(kind,
  stable_id, payload, metadata)` to `df_finance_definer`, stamping `stream_kind`, `stream_id` and
  `campaign_id` on the pending row; **the journal reader** `journal.confirmed_tail(kind, stream_kind,
  stream_id, campaign_id, after_sequence, limit)` with the sequence and in-batch position of every
  entry, and `journal.oldest_unconfirmed_age(kind)` (22.3: replay, `journal_compare` and the
  fail-closed gate need them); consume F's `attempt_outcome_event` through `attempt_outcomes_after`
  (the 24 h failure and F-side unsent cancellations, one path); read `attempt_status` (mapping in
  11.5; `Cancelled` now also covers `VerifiedUnsent`); handle `AlreadyClaimed` and the typed
  `CancelOutcome`; F no longer needs `media_catalog::reject_attempt`; take the personal usage
  statement as an `ExportSource` section (12.6); admit content screens through the fence-free
  account path (11.1).
- **B**: unchanged asks of round 0 except: `apply_processor_state` never raises on a stale state
  (7.3); call `release_account_allocations`, `round_closing_remainder`, `forfeit_promotional` in the
  closure completion (14.5); `closure_blockers` ignores a sub-cent remainder; the refund worker skips
  `manual` and `blocked_by_dispute` lots; operator permissions `FinanceInspect`, `FinanceExport`,
  `ManualAdjustment`, `ManualAdjustmentApprove`, `ManualRefundPayout`, `FinanceClose`; an operator
  bank-statement import; billing pages render only `df_commerce::views::customer`; **two reads for
  the post-restore rebuild** (22.1): `purchase_billing_evidence(purchase_order_ref)` returning the
  billing country and region, and `account_email_grant_key(account_id)`; F's journal holds neither.
- **C**: (1) hosted `begin_call` becomes an async admission through a `FinanceAdmission` port;
  (2) estimates are conservative maxima per priced category covering every declared category;
  (3) `BilledCall::settle`, `not_billed` and drop report under the attempt id through `record_usage`
  with units and a usability flag; (4) Pending routes refused in hosted mode until metered, **Jev
  first** (D21); Unpriced routes need a per-route supplier maximum; (5) move the parser to
  `df_commerce::tariff`; (6) price version label on cost metrics (D20); (7) no float or hard-coded
  price may feed a usage report (H6 5, 6): report units; add `calls` and `image_input` rows and
  context tiers (H6 U1, U4, U7).
- **S1**: `funding` consumer cursor in F's schema (17.2); `campaign_heads_after` catch-up; a fresh
  lock-free `member_snapshot` at admission with `authorized` honored; presets from the pinned
  version; `open_session_funding` before `Start`; closure cancels outgoing transfer proposals.
- **S4**: repoint the POT-06 test to `df_commerce::pot::split_exactly` (the test-local
  `largest_remainder` passes with no F code); when `df-finance` lands, point the system halves at it;
  `spend.rs`'s `SpendCounter.limit` becomes `ExposureLimit` in F's round 1.
- **P**: the customer and operator dashboards use the two view modules only (18.9).
- **Coordinator**: the owner question 28.14 (launch scope with Pending routes refused); migration
  numbers confirmed; `subsystem-architecture.md` rows.
