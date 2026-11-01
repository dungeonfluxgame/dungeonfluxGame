# Creator portrait funding change for review

Owner decision (2026-10-28): charge the creator account balance, with explicit consent and a spending limit, before a character joins a campaign.

The existing finance ledger primarily admits campaign/session work. Implementing the chosen policy requires these specific source changes in migration 0043 and the matching Rust finance contracts:

- Add account code 2150, PurchasedAccountReserved, dimensioned by the existing payer and purchased-credit lot. It holds a reservation in the existing double-entry ledger.
- Add explicit account ownership and mutually exclusive account/session scope checks to existing quote, reservation, and usage records. Reuse the existing account-capable provider attempt and operation receipt.
- Add immutable consent records and account reservation share records. Consent binds the account, draft, job, source/configuration/provider/model/price versions, maximum units, expiry, and spending limit.
- Extend the existing registered finance events, posting allowlists, and database functions for account reservation, settlement, and release. Preserve existing campaign behavior and existing ledger conservation checks.
- Add deferred conservation checks and concurrency/replay tests. Unknown provider outcomes retain their full reserved exposure; a retry cannot silently increase consent or duplicate charges.

Consent remaining capacity is computed from charged amounts and open reservations. There is no second money ledger or fabricated campaign/session.

Requested authorization is to implement these source changes and run migrations against uniquely owned disposable PostgreSQL test databases on port 55471. This does not authorize deploying or migrating a production database or making paid provider calls.

Automatic approval review rejected writing the migration source because of the breadth of changes to shared finance schema and posting/constraint functions. The migration has not been written or applied. The typed policy and review-only Rust patch are prepared; full funding remains blocked pending approval of this exact scope.

## Approval recorded

The user explicitly approved these source changes and disposable-database tests on 2026-10-28. The approved implementation may proceed. Production deployment/migration and paid provider calls remain outside this approval.
