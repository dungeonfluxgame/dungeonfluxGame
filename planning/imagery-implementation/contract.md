# Imagery implementation status and next slice

The full generation and transformation plans remain governing. The attempted full dispatch contract was rejected in independent review; it is not an implementation prerequisite that has passed.

Confirmed boundaries:
- Consumer operation identity binds immutable intent before classification or route selection. Its scope is part of its identity.
- Jobs, catalog publication attempts, logical stages and paid child attempts remain distinct. Every paid child has its own stable finance operation, attempt and effect identities.
- Existing asset publication owns immutable bytes, audience and quota; existing finance owns monetary admission and settlement. No parallel imagery ledger or SQLite gameplay journal is introduced.
- Local and hosted durable adapters use the existing PostgreSQL owner; local accounting preserves current policy.
- A durable send marker must precede external work. Supplier identifiers are recorded after the supplier returns them. Database commits and remote sends are not atomic; uncertain outcomes never authorize automatic repeat dispatch.
- Paid-stage links must include the immutable job, stage, asset and publication generation, and admission replay must bind and verify that lineage.

The independently reviewed next component is [the scoped operation and recipe-freeze store](operation-store.md). It stores durable scoped receipts, shared jobs, per-operation interests and immutable snapshots. It does not implement paid stages, publication or a complete ImageService. Its PostgreSQL acceptance evidence is required before approval.

Still to freeze before paid dispatch: exact catalog Admission construction, production finance/media/fence/member adapters, stage/attempt schema and CAS fences, dispatch authority/config linearization, classification/route result retention, usable/unusable/verified-unsent/unknown settlement mapping, and runtime/service integration. Known-use provenance and old liabilities must remain intact through retries and regeneration.

Main10624f04 contains the independently approved versioned recipe-key encoder. Main6fe8cce4 contains the independently approved native image request/config/transform foundation (86 tests plus public-boundary probes, Clippy and formatting). Provider migration, creator/plate/item/frame consumers, cost reporting, transforms, architecture guards and browser acceptance remain pending. No completion claim is made by this document.
