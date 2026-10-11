# COMPLIANCE.md

## Scope
Repository `extract-payments` mapped to controls for SOC2, HIPAA, and PCI-DSS where applicable.

This mapping identifies intended control ownership; it is not certification or evidence that a control operates in production. Current privacy, consumer-protection, payment-security, and release dependencies are recorded in `docs/compliance/REPOSITORY_AUDIT_2026-10-11.md`.

## Control Mapping
| Control Family | Implementation in Repo |
|---|---|
| SOC2 CC6/CC7 | Pulsar authn/authz boundaries, centralized Quickwit logging, immutable audit topics |
| SOC2 CC8 | Git-tracked architecture/API/compliance artifacts and change plans |
| HIPAA 164.312(b) | Audit controls via Quickwit indexed trace/log records |
| HIPAA 164.312(c) | Integrity checks through event immutability and signed payload patterns |
| PCI-DSS 10 | Centralized security event logging with searchable trace correlation |
| PCI-DSS 7/8 | Role-based topic access and service identity enforcement |

The listed implementations require deployed-configuration, access-review, retention, incident-response, and operating-effectiveness evidence in Nvuto GRC. HIPAA applicability requires a documented data-flow and regulated-entity determination; repository keywords alone are not conclusive.

## Evidence Artifacts
- `PLAN.md`
- `ARCHITECTURE.md`
- `API_SPEC.yaml`
- `eventing/pulsar/topics.yaml`
- `observability/quickwit/index-config.yaml`
