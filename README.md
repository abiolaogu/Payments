# Repository

## Billyronks Sovereign Standardization

- Vertical: **FinTech**
- Benchmark targets: **Stripe, Adyen**
- Event backbone: **Apache Pulsar** (`eventing/pulsar/topics.yaml`)
- Observability/search: **Quickwit** (`observability/quickwit/index-config.yaml`)
- Harvester HCI baseline: **Mayastor/Vitastor-compatible** storage contracts (`infrastructure/kubernetes/harvester/storage-baseline.yaml`)
- Autonomous expansion target: **Autonomous payment routing + real-time fraud graph scoring**

## Security and release status

- The Bookings payment API uses tenant-bound Platform Services workload authentication.
- The default binary's legacy routes require the explicit `payments:legacy-admin` workload permission; they are not approved as consumer APIs.
- Paystack and Flutterwave webhooks fail closed unless the raw request body has a valid provider credential.
- Cross-origin access is disabled unless an origin is explicitly listed in `ALLOWED_ORIGINS`.
- Provider secrets must be injected from Vault/KMS. Never store PAN or sensitive authentication data in this service.

See `docs/compliance/REPOSITORY_AUDIT_2026-10-11.md` for the open Hasura, SpiceDB, PCI, retention, privacy-rights, subscription, accessibility, and jurisdiction release dependencies.
