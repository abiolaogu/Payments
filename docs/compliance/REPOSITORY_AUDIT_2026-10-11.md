# Payments Privacy and Consumer-Protection Audit

**Evidence date:** 2026-10-11

**Baseline commit:** `80ccf6dc5730b7cb604da1dd20327d731dde8aca`
**Status:** Engineering audit and remediation evidence, not legal approval, PCI validation, or production certification.

## Classification and application inventory

`customer-facing` payment platform candidate. The repository contains a default Rust payment API and container, a separately secured Bookings payment-authority API, PostgreSQL migrations, NATS code, payment/refund/wallet/subscription domain code, a web scaffold, and Android, iOS, and Flutter scaffolds.

The Bookings API already derives tenant authority from an active Platform Services workload token and scopes persistence by that tenant. The default container instead launched a legacy administration API. The remediation keeps that legacy surface available only to active workload identities with the explicit `payments:legacy-admin` permission; it does not represent a consumer authorization model.

Canonical ownership remains with StratoID/Authentik for authentication, SpiceDB for authorization, Hasura for product-facing data access, Vault/KMS for provider secrets, NATS JetStream for regional events, OpenObserve/ClickHouse for audit search, OpenTelemetry for telemetry, and Nvuto GRC for findings, evidence, exceptions, and approvals.

## Risk-ranked findings and disposition

| Risk | Baseline evidence | Disposition |
| --- | --- | --- |
| Critical | `src/main.rs` exposed transaction, refund, wallet, top-up, transfer, and payment endpoints without authentication or authorization. | Remediated as a transitional safety boundary: every legacy route now requires an active, unexpired, tenant-bound workload identity carrying `payments:legacy-admin`. Consumer use remains blocked until Hasura and SpiceDB replace this administrative compatibility surface. |
| Critical | `src/main.rs` accepted provider webhooks without signature verification and logged the entire payload. | Remediated: Paystack HMAC-SHA512 and Flutterwave secret verification operate over the raw body; unsigned or invalid requests receive `401`; logs contain only verified provider and event type. Tests cover valid, tampered, missing, and incorrect credentials. |
| High | `CorsLayer::permissive()` allowed browser calls from every origin. | Remediated: no cross-origin access is enabled by default; only explicitly configured `ALLOWED_ORIGINS` may use GET/POST with authorization/content-type headers. |
| High | `web/src/authProvider.ts` fabricated `demo-token` when credentials were absent and persisted bearer material in local storage. | Remediated: missing tokens fail closed; the compatibility token is memory-only and cleared on logout. Production UI remains blocked pending StratoID/Authentik authorization-code flow with secure session handling. |
| High | Wallet transfers debited and credited in separate database operations, allowing partial financial state on failure. | Remediated: both mutations run in one transaction with affected-row validation; invalid/insufficient source and missing destination paths roll back. |
| High | Request validation was not executed on refund, wallet top-up, or transfer handlers; an omitted refund amount became zero. | Remediated: positive amounts are mandatory and validated; self-transfer is rejected; wallet top-up binds the asserted customer to the target wallet. |
| Medium | Raw SQL errors were returned to API callers. | Remediated: persistence failures use a stable generic error without database internals. |
| High/review | `src/domain/aggregates/subscription.rs` can create and renew a subscription without a consent receipt, terms/disclosure version, acknowledgement, reminder, or jurisdiction. | The aggregate is not routed by either API. It must remain dormant until integrated with Nvuto Consent, consumer ownership, required communications, easy cancellation, and product/counsel evidence. The CI scanner keeps this as a human-review item. |
| High/open | Payment methods persist provider tokens and transactions accept arbitrary metadata and customer email, but there is no evidenced data minimisation, field allow-list, retention/deletion/export workflow, or token-vault boundary. | Production activation blocked pending schema/data-flow review, provider tokenization evidence, retention and rights workflows, and Hasura/SpiceDB enforcement. Do not store PAN or sensitive authentication data. |
| High/open | Existing compliance prose names controls but supplies no PCI scope decision, ASV/penetration evidence, key-management evidence, provider responsibility matrix, incident exercise, or operating-effectiveness record. | Treat mappings as intended ownership only. Record approved evidence and exceptions in Nvuto GRC; obtain the appropriate PCI assessment independently. |
| High/open | Web dependency installation reports 41 advisories: 3 moderate, 36 high, and 2 critical. A fresh RustSec advisory fetch was unavailable during verification. | Production release requires SBOM, reachability analysis, tested upgrades, and a successful RustSec review. Do not use a breaking force-upgrade or treat the unavailable audit as a clean result. |
| Medium/review | Web/mobile clients are generic project scaffolds rather than verified payment journeys. | No consumer-flow conclusion is possible. Before release, test disclosures, errors, accessibility, privacy notices, cancellation/refunds, age/audience treatment, and analytics/SDK consent in the actual served clients. |
| Medium/open | Repository documentation/configuration references Pulsar and Quickwit while the approved estate direction is NATS JetStream and OpenObserve/ClickHouse; code already uses `async-nats`. | Preserve payment domain ownership while producing a controlled migration plan for messaging and audit search; do not claim the documented controls operate until deployment evidence exists. |

## Applicability matrix

| Control family | Status | Required evidence before production release |
| --- | --- | --- |
| Privacy / GDPR / UK GDPR / NDPA | Applicable | Data inventory, lawful basis, notice/version evidence, minimisation, retention, rights export/deletion, processors/transfers, and DPIA decision. |
| Children / age safety | Review required | Intended audience and served-jurisdiction decision; child-directed treatment only if applicable. Payment identifiers must not be used as an age-assurance shortcut. |
| Communications privacy / tracking | Review required for served clients | SDK/vendor inventory, consent and withdrawal configuration, masking, retention, and proof that payment fields are excluded. No replay SDK was found in owned source. |
| Marketing | Not evident in this repository | Keep marketing delivery and suppression in the canonical communication service; reassess if payment events trigger campaigns. |
| Subscription / automatic renewal | Applicable only if dormant aggregate is activated | Versioned terms, affirmative Nvuto Consent receipt, acknowledgement, reminders, price-change handling, and easy cancellation. |
| Payment / financial privacy | Applicable | PCI scope and responsibility matrix, provider tokenization, Vault/KMS injection, least privilege, webhook replay handling, reconciliation, audit evidence, and tested incident/retention controls. |
| Accessibility | Applicable to any served web/mobile UI | WCAG 2.2 automated and manual keyboard, screen-reader, focus, reflow, error, and payment/refund-flow evidence. |
| Auditability | Applicable | OpenTelemetry correlation without sensitive payloads, NATS/outbox evidence, immutable Nvuto GRC receipts, and reviewed exceptions. |

## Test and release evidence

- `cargo test --locked --all-targets`: 12 tests passed, including bearer parsing, workload permission decisions, Paystack/Flutterwave webhook verification, and existing Bookings/domain invariants.
- `cargo check --all-targets`: passed with pre-existing dead-code/unused warnings.
- Web: 2 files / 3 tests passed; typecheck, ESLint, and production build passed. The build reports a pre-existing large-chunk warning.
- Dependency installation reports 41 web advisories (3 moderate, 36 high, 2 critical). A subsequent npm audit request and RustSec advisory refresh were unavailable, so no clean dependency conclusion is recorded.
- The tailored privacy scan found zero confirmed blockers and one dormant-subscription review item; the shared estate scanner covered 72 files with zero matched blockers.
- The privacy workflow blocks confirmed demo-token, permissive-CORS, raw-webhook-log, and unsigned-webhook regressions; contextual subscription findings remain advisory.
- Dependency lock data is committed so CI and security review evaluate a reproducible Rust dependency graph.

Release remains blocked for any consumer-facing or general tenant use of the legacy API until Hasura/SpiceDB integration, product-data tenant migration, provider/replay processing, PCI and dependency disposition, data-subject workflows, actual client UX, accessibility, jurisdiction, and counsel evidence are approved in Nvuto GRC. This remediation makes the present compatibility surface fail closed; it does not waive those obligations.
