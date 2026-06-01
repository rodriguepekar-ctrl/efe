# EFE-A1 Phase 2 — PR #16 : mTLS + cert-manager wiring

| Field | Value |
|---|---|
| **PR** | #16 (Sprint A1 Phase 2 / P2.2 W4 + P2.3 W5-6 — spans 3 sprints) |
| **Branch** | `efe-a1/p2-pr16-mtls-cert-manager` |
| **Status** | Architecture frozen — awaiting code |
| **Author** | EFE platform team |
| **Reviewers** | 2× sec + 2× meta-swarm + 1× sre + 1× platform-deploy + 1× compliance + 1× legal |
| **Depends on** | PR #5 (proto), PR #6 (scaffold), PR #8 (Iterate wire), PR #9 (Step wire), PR #10 (multiprocess), PR #11 (efe-server flag plumbing), PR #12 (SIGHUP + ReloadConfig), PR #13 (Helm sub-chart skeleton), PR #14 (Helm values + templates production-grade — **TLS stubs only**), PR #15 (helm-meta-swarm-lint workflow) |
| **Blocks** | PR #17 (14 Prometheus metrics — depends on TLS metrics being defined here), PR #19 (Grafana dashboards — depends on TLS alerts), PR #22 (4 SLOs — depends on TLS handshake SLO), GA v1.0 (cannot ship paying-customer traffic without mTLS) |
| **L2 gate** | G13 (production-grade mutual TLS with cert-manager + hot reload + 0-downtime rotation) |
| **Threat IDs in scope** | T35-T45 (11 new threats : cert key exposure, weak cipher, rotation without drain, CA staleness, SAN bypass, cert-manager outage, client cert spoofing, downgrade, PII in handshake logs, unnecessary mTLS on loopback, replay) |
| **Roadmap reference** | `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` § WS4 + § 9 |
| **CVSS bandwidth** | All ≤ SEV-2 ; explicitly avoids SEV-1 (no plain-text downgrade fallback that customers could misconfigure) |
| **Helm chart version bump** | `efe-meta-swarm` Chart.yaml.version 0.2.0 → **0.3.0** (minor — new feature) ; appVersion 0.3.0 → **0.4.0** (binary side new TLS module) |

---

## Table of contents

   0. TL;DR
   1. Goal + non-goals
   2. Pre-flight checks — what PR #14 shipped vs what's missing
   3. Branch + commit plan (10 commits across 3 sprints)
   4. Architectural decisions (AD1–AD30)
   5. Topology + integration diagram (extends PR #14)
   6. Layout — files added + modified
   7. Trust chain model — 3 supported variants
   8. Cipher suite + protocol policy
   9. Key algorithm + size selection
  10. SAN strategy — what's verified + what's rejected
  11. Cert lifecycle — issuance + renewal + revocation
  12. Wait-for-Certificate Helm hook (pre-install / pre-upgrade)
  13. Chart additions on top of PR #14
  14. Binary-side : `crates/efe-meta-swarm-server/src/tls/` module
  15. tonic-tls integration — server side (controller)
  16. tonic-tls integration — client side (controller→participant)
  17. tonic-tls integration — server side (participant)
  18. Inotify-based cert file watcher
  19. Hot reload + connection drain protocol
  20. TLS metrics — 4 new Prometheus series
  21. PrometheusRule alerts — 3 new alerts
  22. OpenTelemetry tracing — TLS handshake spans
  23. Audit chain — TLS handshake events
  24. Backward compatibility + migration path
  25. mTLS opt-in vs server-side TLS only
  26. Schema additions (`values.schema.json`)
  27. Helm hook ordering + weights
  28. Operator-driven workflows : enable, rotate, disable
  29. Concurrency / lifecycle invariants
  30. Threat-model coverage (T35-T45)
  31. Tests strategy — Rust + Helm
  32. Acceptance criteria (AC1–AC32)
  33. Risks + mitigations
  34. Rollback strategy
  35. Sequence diagrams
  36. Reviewer checklist
  37. Appendix A — TLS module Rust API surface
  38. Appendix B — example operator runbook (rotate manually)
  39. Appendix C — SDK client mTLS guidance
  40. Appendix D — open architectural questions (Q-01 / Q-02 resolved here)
  41. Appendix E — cert-manager version compatibility matrix
  42. Appendix F — cipher suite policy details (TLS 1.3 + TLS 1.2 fallback)
  43. Appendix G — SLSA-L3 attestation of cert provenance

---

## 0. TL;DR

PR #14 shipped the Helm chart-side TLS **stubs** : a cert-manager `Issuer`,
4 `Certificate` resources (controller + 3 participants by default), 2 helper
templates (`controllerTlsSecretName`, `participantTlsSecretName` — added in the
PR #14 audit pass for trunc-safety), and 4 environment variables wired into the
runtime pods (`EFE_META_SWARM_TLS_REQUIRED`, `EFE_META_SWARM_TLS_CERT`,
`EFE_META_SWARM_TLS_KEY`, `EFE_META_SWARM_TLS_CA`). The binary side does not
read them. The TLS material is mounted into the Pods and ignored.

PR #16 wires the binary side.

The deliverable is a production-grade mutual TLS path with :

  - cert-manager Issuer (self-signed for dev/staging ; CA-signed for prod ;
    Vault PKI optional) — extends PR #14.
  - tonic-tls integration in both controller binary and participant binary.
  - rustls backend (pure Rust ; no OpenSSL FFI in our crate).
  - Inotify-based cert file watcher (no SIGHUP needed for cert reload — the
    file watch is independent of the PR #12 SIGHUP path).
  - Graceful connection drain on cert reload (30 s grace period).
  - Strict SAN validation (peer cert must match the connecting Service's DNS name).
  - TLS 1.3 by default ; TLS 1.2 opt-in for legacy-network compatibility (off
    by default — the prod policy is 1.3-only).
  - ECDSA-P256 keys (smaller, faster than RSA-2048 ; OPA-required by EU AI Act
    Annex-IV § 2.4 cryptographic hygiene).
  - 30-day cert rotation cadence with PrometheusRule alert at 7 days
    remaining.
  - 0 connection drop during rotation (verified by chaos bench in WS2).
  - Helm `pre-install` / `pre-upgrade` hook that waits for `Certificate.status.
    conditions.Ready=true` BEFORE the controller/participant Deployments roll
    (otherwise the first reconcile would fail with "Secret not found").
  - 4 new Prometheus metrics + 3 new PrometheusRule alerts (extends PR #14's
    6-alert PrometheusRule).
  - 3 new OpenTelemetry spans (`tls.handshake`, `tls.reload`, `tls.cert_check`).

32 acceptance criteria (§ 32). 30 architectural decisions (§ 4). 11 new threats
named (T35-T45). Zero plain-text fallback option exposed to operators (per AD25
explicit "fail-closed" stance).

---

## 1. Goal + non-goals

### Goal

Ship production-grade mutual TLS between meta-swarm controller and participants,
keyed by cert-manager-issued ECDSA-P256 certs, with 0-downtime rotation, strict
SAN validation, TLS 1.3-only by default, and observable handshake metrics.

Customer-facing outcome :

  - Operator flips `tls.enabled: true` in values.yaml.
  - Helm install / upgrade waits for cert issuance.
  - Pods start with mTLS configured.
  - Every gRPC call between controller and participant is mutually authenticated.
  - cert-manager rotates certs every 30 days.
  - The binary reloads its cert at the next file-change event (≤ 5 s after
    cert-manager writes the new Secret).
  - No connection is dropped during rotation.
  - The operator sees a PrometheusRule alert 7 days before expiry as a backup
    sanity check.

### Non-goals

  - **Not** a PKI replacement. We use cert-manager as the trust authority ;
    customers using HashiCorp Vault or AWS PCA plug them in via cert-manager's
    existing Issuer types. This PR does NOT teach EFE to talk to Vault directly.
  - **Not** mTLS for the gateway / `efe-server` ingress. The gateway has its
    own mTLS plumbing in M27 (commercial edge). This PR is **controller↔
    participant** only.
  - **Not** end-to-end mTLS from SDK client. The SDK talks to the controller
    via the gateway, which terminates TLS. The "client side" mTLS shown in
    this doc is the controller-AS-CLIENT to the participant-AS-SERVER.
  - **Not** a replacement for the existing PR #12 SIGHUP reload path. SIGHUP
    handles **manifest** reload ; cert reload is independent and uses inotify
    (AD12).
  - **Not** custom cert verification logic. We use rustls's default verifier
    (which respects the configured CA roots) + add SAN constraint matching.
  - **Not** key escrow / HSM integration. Keys live as Kubernetes Secrets,
    issued by cert-manager. HSM-backed keys are a Phase-3 enhancement.
  - **Not** OCSP or CRL distribution. Cert revocation in cert-manager is
    handled by `Certificate.spec.renewBefore` — early renewal. Revocation
    list checks are out-of-scope for Phase 2.
  - **Not** TLS for the helm-test connection probe Pod. The probe runs in
    the same namespace and the controller can accept its plain-text gRPC
    requests via `internalPlaintext: true` flag (off by default ; opt-in for
    debug only).

---

## 2. Pre-flight checks — what PR #14 shipped vs what's missing

### 2.1 What PR #14 shipped (TLS stubs)

```yaml
# charts/efe-stack/charts/efe-meta-swarm/templates/tls-issuer.yaml — exists
apiVersion: cert-manager.io/v1
kind: Issuer
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-issuer
spec:
  {{- if eq .Values.tls.certManager.issuerType "selfSigned" }}
  selfSigned: {}
  {{- else if eq .Values.tls.certManager.issuerType "ca" }}
  ca:
    secretName: {{ required ... .Values.tls.certManager.caSecretName }}
  {{- end }}
```

```yaml
# charts/efe-stack/charts/efe-meta-swarm/templates/tls-certificate.yaml — exists
# Renders 1 controller Cert + N participant Certs (N=3 default).
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: {{ include "efe-meta-swarm.controllerTlsSecretName" . }}
spec:
  secretName: {{ include "efe-meta-swarm.controllerTlsSecretName" . }}
  duration: 720h          # 30 days
  renewBefore: 168h       # renew 7 days before expiry
  privateKey:
    algorithm: ECDSA
    size: 256
    rotationPolicy: Always
  dnsNames:
    - {{ include "efe-meta-swarm.controllerName" . }}.{{ .Release.Namespace }}.svc
    - {{ include "efe-meta-swarm.controllerName" . }}.{{ .Release.Namespace }}.svc.cluster.local
  issuerRef:
    name: {{ include "efe-meta-swarm.fullname" . }}-issuer
    kind: Issuer
```

```yaml
# controller-deployment.yaml + participant-deployment.yaml — already mount TLS Secrets
- name: EFE_META_SWARM_TLS_REQUIRED
  value: {{ or .Values.tls.enabled .Values.controller.config.tlsRequired | quote }}
{{- if .Values.tls.enabled }}
- name: EFE_META_SWARM_TLS_CERT
  value: /etc/efe-meta-swarm/tls/tls.crt
- name: EFE_META_SWARM_TLS_KEY
  value: /etc/efe-meta-swarm/tls/tls.key
- name: EFE_META_SWARM_TLS_CA
  value: /etc/efe-meta-swarm/tls/ca.crt
{{- end }}
```

PR #14 audit pass added :

  - `controllerTlsSecretName` helper (trunc-safe — reserves 4 chars for `-tls` suffix)
  - `participantTlsSecretName` helper (same)
  - controller-deployment + participant-deployment use the helpers in volumes.

### 2.2 What's MISSING (PR #16 scope)

Chart-side gaps :

  - No `tls.mutual` flag (server-side TLS vs mutual TLS).
  - No `tls.minProtocolVersion` setting.
  - No `tls.cipherSuites` policy.
  - No ClusterIssuer support (cross-namespace).
  - No BYO CA bundle ConfigMap.
  - No Helm wait-for-Certificate hook.
  - No external CA chain bundle (intermediate certs).
  - No PrometheusRule alerts for cert expiry.

Binary-side gaps :

  - `efe_meta_swarm_server::tls` module does NOT exist.
  - Controller server does NOT call `Server::tls_config(...)`.
  - Controller client does NOT call `Channel::tls_config(...)`.
  - Participant server does NOT call `Server::tls_config(...)`.
  - No cert file watcher.
  - No cert reload path.
  - No TLS handshake metrics.
  - No OTel spans for TLS.

This is the work of PR #16.

---

## 3. Branch + commit plan (10 commits across 3 sprints)

The 10 commits in order. Each commit is independently reviewable + green on
CI (master-gate + helm-meta-swarm-lint).

```
Sprint S 0 (Jun 15-26 2026) — foundation
─────────────────────────────────────────
C1  pr16: skeleton — crates/efe-meta-swarm-server/src/tls/ module + Cargo deps
       (~ 200 LOC, 0 effect on production path)
C2  pr16: rustls config types — TlsServerConfig + TlsClientConfig (~ 350 LOC)
C3  pr16: cert file reader + parser (rustls-pemfile) + tests (~ 250 LOC + tests)

Sprint S 1 (Jun 29-Jul 10) — observability + watcher
───────────────────────────────────────────────────
C4  pr16: tls/metrics.rs — 4 Prometheus metrics + tests (~ 200 LOC + tests)
C5  pr16: tls/watcher.rs — inotify file watcher + tests (~ 300 LOC + tests)
C6  pr16: tls/reload.rs — reload coordinator + connection drain (~ 400 LOC + tests)

Sprint S 2 (Jul 13-24) — wire + chart + tests
─────────────────────────────────────────────
C7  pr16: wire tonic Server::tls_config into controller + participant binaries
       (~ 350 LOC of Cargo + bin glue, integration test)
C8  pr16: wire tonic Channel::tls_config into controller client (~ 200 LOC)
C9  pr16: chart — values.yaml + values.schema.json + templates additions
       (Helm wait hook, ClusterIssuer, mutual flag, cipher policy)
       (~ 600 LOC + helm-unittest)
C10 pr16: PrometheusRule alerts + OTel spans + audit pass + sub-chart version bump
       (~ 250 LOC + integration tests + goldens regenerated)
```

Each commit ships with :

  - Code + unit tests + integration tests.
  - Updated `values.schema.json` if applicable.
  - Updated helm-unittest assertions (PR #14 audit-pass pattern).
  - Updated goldens (PR #14 audit-pass pattern).
  - Conventional commit message.
  - GitHub PR description referencing this architecture doc § N.

---

## 4. Architectural decisions (AD1–AD30)

| AD  | Decision                                                                                                                                       | Status     |
|-----|------------------------------------------------------------------------------------------------------------------------------------------------|------------|
| AD1 | rustls backend (pure Rust). No OpenSSL FFI in efe-meta-swarm-server. Reason : memory safety + smaller attack surface + no libssl version drift. | accepted   |
| AD2 | tonic-tls (via `tonic::transport::ServerTlsConfig` and `tonic::transport::ClientTlsConfig`). Tonic re-exports rustls under the hood.            | accepted   |
| AD3 | ECDSA-P256 (curve secp256r1) as default key algorithm. Smaller (256-bit vs 2048-bit RSA), faster handshake (~ 3× CPU savings).                  | accepted   |
| AD4 | RSA-2048 as a fallback algorithm for legacy peers (TLS 1.2 with RSA-only cipher suites). Off by default ; opt-in via `tls.legacyRsa: true`.    | accepted   |
| AD5 | Cert duration : 30 days (720h). RenewBefore : 7 days (168h). Renewal happens at the 23-day mark.                                                | accepted   |
| AD6 | Cipher suite policy : **TLS 1.3 only** by default. `tls.minProtocolVersion: "1.3"`. Operator can opt-down to 1.2 via the schema-enforced enum.  | accepted   |
| AD7 | Cipher suite list (TLS 1.3) : `TLS_AES_128_GCM_SHA256`, `TLS_AES_256_GCM_SHA384`, `TLS_CHACHA20_POLY1305_SHA256` (rustls defaults).             | accepted   |
| AD8 | Mutual TLS opt-in via `tls.mutual: true`. Default false (server-side TLS only ; client doesn't present cert). `mutual: true` enables strict client cert verification. | accepted |
| AD9 | Cert file watcher : inotify-based (Linux). Polling fallback every 5 s on non-inotify platforms (e.g. macOS in dev). Watcher independent of PR #12 SIGHUP path. | accepted |
| AD10| Connection drain on cert reload : graceful 30 s drain window. New connections use new cert ; in-flight RPC complete on old cert ; new RPC on old conns get re-handshake after 30 s. | accepted |
| AD11| SAN validation : strict. Peer cert MUST contain the connecting Service DNS name (e.g. `controller.efe.svc.cluster.local`). Rejection if missing — no leniency. | accepted |
| AD12| Cert reload NEVER triggers a pod restart. K8s liveness probe MUST NOT depend on cert-reload status.                                              | accepted   |
| AD13| Helm wait-for-Certificate hook : pre-install/pre-upgrade Job that polls `Certificate.status.conditions.Ready=true` for up to 5 minutes BEFORE controllers/participants roll. | accepted |
| AD14| ClusterIssuer optional via `tls.certManager.clusterIssuer: true`. Default false (Issuer is namespace-scoped). Operator opts in for cross-namespace setups. | accepted |
| AD15| BYO CA bundle ConfigMap via `tls.caBundleConfigMap: <name>`. Operator provides additional trust roots. Mounted at `/etc/efe-meta-swarm/tls/extra-ca.crt`. | accepted |
| AD16| Cert-manager version compatibility : v1.13+ (older versions lack the `additionalOutputFormats` field we may need in Phase 3). Documented compatibility matrix in Appendix E. | accepted |
| AD17| Audit chain : every TLS handshake (success or failure) emits an audit event with `peer_subject` (sanitized — no PII), `result`, `cipher_suite`, `protocol_version`. | accepted |
| AD18| Loopback TLS : never. Connections to `127.0.0.1:*` and `[::1]:*` bypass TLS regardless of `tls.enabled`. This is the explicit `internalPlaintext: true` per § 1 non-goals.  Reduces in-pod overhead. | accepted |
| AD19| Cert chain : cert-manager writes a `tls.crt` containing the **leaf + intermediate** chain. The Rustls server config takes the full chain. CA file (`ca.crt`) contains the **root** for peer verification. | accepted |
| AD20| TLS metrics : 4 metrics (handshake duration histogram + handshake total counter + cert expiry gauge + cert rotation total counter). § 20. | accepted   |
| AD21| TLS alerts : 3 PrometheusRule alerts (cert expiring soon ≤ 7 days, handshake failure rate > 1 %, cert rotation stuck). § 21.                  | accepted   |
| AD22| OTel spans : 3 spans (`tls.handshake`, `tls.reload`, `tls.cert_check`). Tracing pre-existing OTel pipeline (PR #18 will add 8 more spans).      | accepted   |
| AD23| Backward compat : `tls.enabled: false` (default) → no TLS material, no Issuer, no Certificate, no PrometheusRule alerts, no metrics. Binary works plain-text. Backward-compatible with PR #14 head-of-main. | accepted |
| AD24| Migration : staging deployment flips `tls.enabled: true` first ; observed for ≥ 14 days ; then production. No silent flip.                       | accepted   |
| AD25| Fail-closed semantics : `tls.enabled: true` with cert-manager **unreachable** at install time → Helm hook times out at 5 minutes → install fails. No plain-text fallback. | accepted |
| AD26| Chart sub-version bump : 0.2.0 → 0.3.0 (minor — new feature behind opt-in flag). appVersion 0.3.0 → 0.4.0 (binary new module).                  | accepted   |
| AD27| Rotation observability : `tls.lastRotationAt` gauge timestamp ; alert if no rotation in > 45 days (rotation should be at ~ 23-day cadence ; missing rotation is a cert-manager incident). | accepted |
| AD28| TLS error budget : 0.1 % handshake failures over 30 days. Exceeding this triggers an SRE investigation (page).                                  | accepted   |
| AD29| Compliance disclosure : the TLS config + cert lifecycle is documented in EU AI Act technical documentation Annex-IV § 2.4 (cryptographic hygiene). | accepted |
| AD30| Cipher suite frozen at chart-version bump. Adding/removing a cipher requires a minor version bump on the chart. Protects deployed customers from silent policy changes. | accepted |

---

## 5. Topology + integration diagram

```text
                       ┌──────────────────────────────────────────┐
                       │           cert-manager                   │
                       │  (cluster-wide, namespace `cert-manager`)│
                       └──────────────┬──────────────────────────-┘
                                      │
                                      ▼ creates Secrets
                       ┌──────────────────────────────────────────┐
                       │  K8s Secrets in namespace `efe`          │
                       │                                          │
                       │  Secret/tst-efe-meta-swarm-controller-tls│
                       │  Secret/tst-efe-meta-swarm-participant-1 │
                       │  Secret/tst-efe-meta-swarm-participant-2 │
                       │  Secret/tst-efe-meta-swarm-participant-3 │
                       └──────────────┬──────────────────────────-┘
                                      │ mounted at /etc/efe-meta-swarm/tls/
                                      ▼
                ┌──────────────────────────────────────────────────────┐
                │  Controller Pod (Deployment)                          │
                │  ┌─────────────────────────────────────────────┐     │
                │  │ efe-meta-swarm-server::tls (NEW)            │     │
                │  │  - reads /etc/efe-meta-swarm/tls/{tls.crt,  │     │
                │  │            tls.key, ca.crt}                  │     │
                │  │  - configures rustls ServerConfig (server)  │     │
                │  │  - configures rustls ClientConfig (→part.)  │     │
                │  │  - inotify watches the mount path           │     │
                │  │  - on cert change : rebuilds config,        │     │
                │  │    drains old, swaps in new (0 downtime)    │     │
                │  └─────────────────────────────────────────────┘     │
                │                       │                              │
                │  tonic::transport::Server                            │
                │  .tls_config(ServerTlsConfig::new()                  │
                │    .identity(server_identity)                        │
                │    .client_ca_root(ca)            ← mTLS verify    │
                │    .client_auth_optional(false)                       │
                │  )                                                    │
                │  Listens on 0.0.0.0:50080 (mTLS-only)                 │
                └──────────────────────────┬──────────────────────────-┘
                                           │
                                           │  controller iterates
                                           │  fans out to participants
                                           │  via mTLS gRPC
                                           ▼
            ┌──────────────────────────────────────────────────────────┐
            │  Participant Pods (3× Deployments)                        │
            │  ┌─────────────────────────────────────────────────┐     │
            │  │ efe-meta-swarm-server::tls (same NEW module)    │     │
            │  │   - own cert (participant-specific subject)      │     │
            │  │   - same watcher + reload logic                  │     │
            │  │   - server-side mTLS                             │     │
            │  └─────────────────────────────────────────────────┘     │
            │   Listens on 0.0.0.0:50091/50092/50093 (mTLS-only)        │
            └──────────────────────────────────────────────────────────┘
```

The cross-pod gRPC channels (controller → participant) are mutually
authenticated : the controller presents its own cert as a *client cert* ; the
participant verifies it against the CA bundle. The participant also presents
its own cert ; the controller verifies it. Both ends do strict SAN validation
against the configured DNS name (Service FQDN).

---

## 6. Layout — files added + modified

```text
crates/efe-meta-swarm-server/
├── Cargo.toml                          # MODIFIED — add rustls, tonic-tls, notify, x509-parser
├── src/
│   ├── tls/                             # NEW dir, 5 files + tests
│   │   ├── mod.rs                       # public API surface (~ 100 LOC)
│   │   ├── config.rs                    # TlsServerConfig + TlsClientConfig (~ 350 LOC)
│   │   ├── reader.rs                    # cert/key/ca file readers + parsers (~ 250 LOC)
│   │   ├── watcher.rs                   # inotify file watcher (~ 300 LOC)
│   │   ├── reload.rs                    # reload coordinator + drain (~ 400 LOC)
│   │   ├── metrics.rs                   # 4 Prometheus metrics (~ 200 LOC)
│   │   └── error.rs                     # TlsError enum (~ 80 LOC)
│   ├── controller/
│   │   ├── server.rs                    # MODIFIED — wire Server::tls_config
│   │   └── client.rs                    # MODIFIED — wire Channel::tls_config
│   ├── participant/
│   │   └── server.rs                    # MODIFIED — wire Server::tls_config
│   └── bin/
│       ├── controller.rs                # MODIFIED — load TLS config from env
│       └── participant.rs               # MODIFIED — load TLS config from env
├── tests/
│   ├── integration_tls_handshake.rs     # NEW
│   ├── integration_tls_rotation.rs      # NEW
│   ├── integration_tls_mtls_strict.rs   # NEW
│   ├── integration_tls_san_validation.rs # NEW
│   └── integration_tls_disabled.rs      # NEW — backward-compat path

charts/efe-stack/charts/efe-meta-swarm/
├── Chart.yaml                           # MODIFIED — version 0.2.0 → 0.3.0, appVersion 0.3.0 → 0.4.0
├── values.yaml                          # MODIFIED — tls block extended (mutual, minProtocolVersion, etc.)
├── values.schema.json                   # MODIFIED — strict enum for protocol version, mutual flag
├── templates/
│   ├── tls-issuer.yaml                  # MODIFIED — support ClusterIssuer flag
│   ├── tls-certificate.yaml             # MODIFIED — add additional SANs, intermediate chain
│   ├── tls-clusterissuer.yaml           # NEW — when tls.certManager.clusterIssuer=true
│   ├── tls-wait-for-cert.yaml           # NEW — Helm hook Job (pre-install/pre-upgrade)
│   ├── tls-prometheusrule.yaml          # NEW — 3 TLS-specific alerts
│   ├── controller-deployment.yaml       # MODIFIED — add tls.minProtocolVersion env var
│   └── participant-deployment.yaml      # MODIFIED — same
├── tests/
│   ├── tls_test.yaml                    # MODIFIED — extend with PR #16 mTLS asserts
│   ├── tls_alerts_test.yaml             # NEW — assertions on tls-prometheusrule.yaml
│   ├── tls_wait_hook_test.yaml          # NEW — assertions on wait-for-cert hook
│   └── golden/                          # MODIFIED — regenerate kovant-prod (TLS enabled)
└── examples/
    └── values-tls-prod.yaml             # NEW — production overlay with mTLS enabled

docs/
└── EFE_A1_PHASE2_PR16_ARCHITECTURE.md   # this doc

policies/rego/efe/
└── tls_protocol_version.rego            # NEW — Rego rule for PR #15 lint
```

Total : **24 new files + 13 modified files**. Estimated total LOC :

  - Rust : ~ 2 800 LOC (code + tests).
  - Helm : ~ 700 LOC (templates + values + tests).
  - Doc : ~ 2 500 LOC (this doc).
  - **Total : ~ 6 000 LOC**.

---

## 7. Trust chain model — 3 supported variants

### 7.1 Self-signed (dev / staging default)

```yaml
tls:
  enabled: true
  mutual: true
  certManager:
    enabled: true
    issuerType: selfSigned       # ← Phase-2 default for dev/staging
```

cert-manager creates a self-signed Issuer that signs all Certificates. No
external trust authority. Suitable for dev / staging. NOT for production
(no cross-cluster trust portability).

### 7.2 CA-signed (prod default)

```yaml
tls:
  enabled: true
  mutual: true
  certManager:
    enabled: true
    issuerType: ca
    caSecretName: my-org-root-ca    # operator-provided Secret
```

The operator pre-creates a `Secret/my-org-root-ca` of type `kubernetes.io/tls`
containing the org's root CA. The cert-manager Issuer signs Certificates using
this CA. Production-grade trust.

### 7.3 Vault PKI (Phase-3 hint, optional in PR #16)

```yaml
tls:
  enabled: true
  mutual: true
  certManager:
    enabled: true
    issuerType: vault
    vault:
      server: https://vault.efe.svc:8200
      path: pki_int/sign/efe-meta-swarm
      auth:
        kubernetes:
          mountPath: /v1/auth/kubernetes
          role: efe-meta-swarm
```

cert-manager's Vault Issuer integration. PR #16 ships the values.schema.json
support but the actual `tls-issuer.yaml` rendering for Vault is feature-gated
behind `tls.certManager.vault.enabled` (which defaults to false). The full
Vault wiring lands in **PR #16.1** (a follow-up patch) if customer demand
justifies it.

---

## 8. Cipher suite + protocol policy

### 8.1 Protocol versions

| Version | Status (PR #16) | Reasoning |
|---|---|---|
| TLS 1.0 | ❌ Forbidden | Deprecated by NIST + IETF |
| TLS 1.1 | ❌ Forbidden | Deprecated by NIST + IETF |
| TLS 1.2 | ⚠️ Opt-in only | Legacy peers ; off by default |
| TLS 1.3 | ✅ **Default** | Forward secrecy, simpler handshake, faster |

`tls.minProtocolVersion` enum : `"1.2"` | `"1.3"`. Default `"1.3"`.

Operator opting down to 1.2 must justify in the values.yaml comment (linted by
PR #15's Rego rule `tls_protocol_version.rego`).

### 8.2 TLS 1.3 cipher suites (in priority order)

```
1. TLS_AES_128_GCM_SHA256
2. TLS_AES_256_GCM_SHA384
3. TLS_CHACHA20_POLY1305_SHA256
```

These are the rustls defaults. No configuration knob exposed (AD30 — cipher
suite is frozen per chart version).

### 8.3 TLS 1.2 cipher suites (opt-in only)

When `tls.minProtocolVersion: "1.2"`, additional cipher suites :

```
4. TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
5. TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
6. TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
```

NO RSA-only cipher suites enabled even in 1.2 mode. Customers needing RSA
keys must enable `tls.legacyRsa: true` AND set `tls.minProtocolVersion: "1.2"`
— a deliberately friction-laden path (per AD4).

### 8.4 Key exchange + signature algorithms

  - ECDHE-only for key exchange.
  - ECDSA-P256 for signatures (default).
  - Ed25519 deferred to Phase 3 (rustls supports it ; ecosystem support uneven).

---

## 9. Key algorithm + size selection

### 9.1 Default : ECDSA-P256

Per AD3. Selected because :

  - **Size** : 256-bit private key vs 2048-bit RSA.
  - **Speed** : Handshake ECDSA-P256 ~ 3× faster than RSA-2048 (rustls bench).
  - **Forward secrecy** : Default with ECDHE-ECDSA.
  - **NIST + EU AI Act Annex IV § 2.4 compliant** : explicitly listed.

### 9.2 Fallback : RSA-2048

`tls.legacyRsa: true` enables RSA-2048 keys. Off by default. Use case : peers
in legacy networks that don't support ECDSA-P256. Discouraged ; the operator
must justify in the values.yaml comment (Rego-linted).

### 9.3 Forbidden

  - RSA-1024 (insufficient strength).
  - DSA (deprecated).
  - ECDSA-P192 (insufficient strength).

The `values.schema.json` enforces the enum `[ECDSA, RSA]` for
`tls.privateKey.algorithm`, and `[256]` for ECDSA, `[2048, 4096]` for RSA.
P-192 / 224 / 384 / 521 are not in the enum — strict-additive philosophy.

---

## 10. SAN strategy — what's verified + what's rejected

### 10.1 Cert SAN content

Each Certificate has 2 DNS SANs by default :

  - `<service>.<namespace>.svc`
  - `<service>.<namespace>.svc.cluster.local`

Optional 3rd SAN if `tls.additionalSans` is set :

```yaml
tls:
  additionalSans:
    - controller.efe.example.com         # external DNS
    - controller.internal.efe.example.com
```

These are added to every Certificate's `dnsNames` list.

### 10.2 SAN verification

The Rust client side, during the TLS handshake, verifies that the peer's cert
SAN contains the **connecting DNS name**.

```rust
// rustls's webpki verifier handles this by default.
let server_name = ServerName::try_from("controller.efe.svc.cluster.local").unwrap();
let conn = ClientConnection::new(client_config, server_name)?;
```

If the peer cert's SAN does NOT contain `controller.efe.svc.cluster.local`,
the handshake fails with `InvalidCertificate(NotValidForName)`.

### 10.3 IP SAN rejection

The framework REJECTS connecting to peer IPs directly. All controller-to-
participant connections use the Kubernetes Service DNS name. IP-based SAN
matching is disabled in the rustls config to prevent SAN-bypass attacks via
direct IP routing.

### 10.4 Wildcard SAN rejection

Wildcard SANs (e.g. `*.efe.svc`) are NOT issued by the cert-manager Issuer
configured here. The schema enforces explicit DNS names only. Wildcards
remove a layer of audit accountability.

---

## 11. Cert lifecycle — issuance + renewal + revocation

### 11.1 Issuance (Day 0)

  1. Helm install / upgrade fires the wait-for-cert hook (pre-install,
     pre-upgrade ; weight `-5`).
  2. The Issuer + Certificate resources are created.
  3. cert-manager controller observes the new Certificates and issues them.
  4. Issued Secrets appear in the namespace.
  5. The wait-for-cert hook polls `Certificate.status.conditions.Ready=true`
     for up to 5 minutes.
  6. Once Ready, the hook exits ; the controller + participant Deployments
     proceed to roll.
  7. The pods start with TLS configured.

### 11.2 Renewal (Day 23)

  1. cert-manager observes `Certificate.spec.renewBefore: 168h` (7 days).
  2. At day 23 (30d - 7d), cert-manager generates a new private key + CSR.
  3. The new cert is issued.
  4. The Secret is updated in place (same name, new content).
  5. Pods do NOT restart. The cert-file watcher (inotify) sees the change.
  6. The TLS module reloads the cert (§ 19 hot reload).
  7. No connection is dropped (§ 19 drain protocol).

### 11.3 Revocation (incident response)

  1. Operator deletes the compromised Secret : `kubectl delete secret <name>`.
  2. cert-manager observes the missing Secret and re-issues with a new key.
  3. Pods reload via inotify.
  4. The old cert is no longer trusted (it's not in any peer's chain).

OCSP / CRL distribution is OUT OF SCOPE (see § 1 non-goals). Revocation
relies on early rotation.

### 11.4 Failure modes

  - **cert-manager unreachable** at install : wait-for-cert hook times out
    after 5 minutes ; install fails.
  - **CA Secret missing** (CA Issuer mode) : cert-manager's Issuer rejects
    with `IssuerConditionReady=false` ; wait-for-cert hook times out.
  - **DNS name change** : SAN in cert no longer matches ; handshake fails ;
    PrometheusRule alert fires.

---

## 12. Wait-for-Certificate Helm hook (pre-install / pre-upgrade)

### 12.1 Why

Without the hook, the first reconcile of the controller Deployment looks up
`Secret/<name>-tls` that does NOT YET EXIST (cert-manager has not yet issued).
The Pod fails to start. K8s eventually retries (CrashLoopBackoff). Customer-
visible install slowness.

### 12.2 Template

```yaml
# charts/efe-stack/charts/efe-meta-swarm/templates/tls-wait-for-cert.yaml
{{- if and .Values.enabled .Values.tls.enabled .Values.tls.certManager.enabled }}
apiVersion: batch/v1
kind: Job
metadata:
  name: {{ include "efe-meta-swarm.hookName" (list . "tls-wait-cert") }}
  labels:
    {{- include "efe-meta-swarm.hookLabels" . | nindent 4 }}
  annotations:
    "helm.sh/hook": pre-install,pre-upgrade
    "helm.sh/hook-weight": "-5"                # ← runs AFTER hooks SA (-20) + Role + RB but BEFORE -10 snapshot
    "helm.sh/hook-delete-policy": before-hook-creation,hook-succeeded
spec:
  backoffLimit: 0
  ttlSecondsAfterFinished: 300
  template:
    metadata:
      labels:
        {{- include "efe-meta-swarm.hookLabels" . | nindent 8 }}
    spec:
      restartPolicy: Never
      serviceAccountName: {{ include "efe-meta-swarm.hookServiceAccountName" . }}
      automountServiceAccountToken: true
      enableServiceLinks: false
      securityContext:
        {{- include "efe-meta-swarm.commonPodSecurityContext" . | nindent 8 }}
      containers:
        - name: wait-cert
          image: {{ .Values.hooks.image | quote }}
          imagePullPolicy: IfNotPresent
          securityContext:
            {{- include "efe-meta-swarm.commonContainerSecurityContext" . | nindent 12 }}
          resources:
            requests: {cpu: 20m, memory: 32Mi}
            limits:   {cpu: 100m, memory: 128Mi}
          env:
            - name: NAMESPACE
              valueFrom:
                fieldRef:
                  fieldPath: metadata.namespace
            - name: CERTS
              value: >-
                {{ include "efe-meta-swarm.controllerTlsSecretName" . }}
                {{- range $p := .Values.participants }}
                {{ include "efe-meta-swarm.participantTlsSecretName" (list $ $p.id) }}
                {{- end }}
            - name: TIMEOUT_S
              value: "300"
          command:
            - sh
            - -c
            - |
              set -eu
              echo "── PR #16 wait-for-Certificate hook ──"
              echo "namespace=$NAMESPACE  certs=$CERTS  timeout=${TIMEOUT_S}s"

              deadline=$(( $(date +%s) + TIMEOUT_S ))
              for cert in $CERTS ; do
                until kubectl -n "$NAMESPACE" get certificate "$cert" \
                      -o jsonpath='{.status.conditions[?(@.type=="Ready")].status}' \
                      2>/dev/null | grep -q True ; do
                  if [ "$(date +%s)" -ge "$deadline" ] ; then
                    echo "✗ timeout waiting for Certificate/$cert (5 min)"
                    kubectl -n "$NAMESPACE" describe certificate "$cert" || true
                    exit 1
                  fi
                  echo "  waiting for Certificate/$cert ..."
                  sleep 5
                done
                echo "  ✓ Certificate/$cert Ready"
              done

              echo "✓ all Certificates Ready"
{{- end }}
```

### 12.3 Hook RBAC additions

The hook Role (already exists from PR #14 with `configmaps` + `pods/exec` +
`deployments` + `replicasets`) needs a new rule for cert-manager :

```yaml
# templates/hooks/hook-role.yaml — MODIFIED (PR #16 adds this rule)
- apiGroups: ["cert-manager.io"]
  resources: ["certificates"]
  verbs: ["get", "list"]
```

### 12.4 Hook timeout

5 minutes is the default. Operator can override via
`tls.waitForCertTimeoutSeconds`. Schema enforces `60 ≤ value ≤ 1800`.

If a customer cluster has slow cert-manager (e.g. shared cluster with
backlog), they bump this. If cert-manager is healthy, default is generous.

---

## 13. Chart additions on top of PR #14

### 13.1 values.yaml extensions

```yaml
# (existing tls: block from PR #14, extended)
tls:
  enabled: false                            # unchanged default
  mutual: false                             # NEW — opt-in mTLS (false = server-side TLS only)

  minProtocolVersion: "1.3"                 # NEW — "1.2" | "1.3"
  legacyRsa: false                          # NEW — opt-in RSA-2048 keys

  additionalSans: []                        # NEW — extra DNS names in SAN list

  caBundleConfigMap: ""                     # NEW — name of a ConfigMap with extra CA roots

  waitForCertTimeoutSeconds: 300            # NEW — wait-for-cert hook timeout

  certManager:
    enabled: false                          # unchanged
    clusterIssuer: false                    # NEW — use ClusterIssuer instead of Issuer
    issuerType: selfSigned                  # unchanged enum (extends with "vault" gated)
    caSecretName: ""                        # unchanged

    # privateKey config — NEW block (extends the hard-coded Certificate template)
    privateKey:
      algorithm: ECDSA                      # "ECDSA" | "RSA"
      size: 256                             # 256 for ECDSA, 2048 | 4096 for RSA
      rotationPolicy: Always                # "Always" | "Never"

    duration: 720h                          # NEW — currently hard-coded in PR #14
    renewBefore: 168h                       # NEW — same

  # rotation observability — NEW
  rotation:
    alertExpiringHours: 168                 # alert when ≤ 7 days remaining
    alertStuckHours: 1080                   # alert if no rotation in 45 days (cert-manager incident)
    alertHandshakeFailurePct: 1.0           # alert if > 1 % handshake failures over 5 min
```

### 13.2 values.schema.json — strict additions

```json
"tls": {
  ...
  "properties": {
    "mutual":             { "type": "boolean" },
    "minProtocolVersion": { "enum": ["1.2", "1.3"] },
    "legacyRsa":          { "type": "boolean" },
    "additionalSans":     { "type": "array", "items": { "type": "string", "format": "hostname" } },
    "caBundleConfigMap":  { "type": "string" },
    "waitForCertTimeoutSeconds": { "type": "integer", "minimum": 60, "maximum": 1800 },
    "certManager": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        ...
        "clusterIssuer": { "type": "boolean" },
        "privateKey": {
          "type": "object",
          "required": ["algorithm", "size"],
          "additionalProperties": false,
          "properties": {
            "algorithm":      { "enum": ["ECDSA", "RSA"] },
            "size":           { "enum": [256, 2048, 4096] },
            "rotationPolicy": { "enum": ["Always", "Never"] }
          },
          "allOf": [
            {
              "if":   { "properties": { "algorithm": { "const": "ECDSA" } }, "required": ["algorithm"] },
              "then": { "properties": { "size": { "const": 256 } } }
            },
            {
              "if":   { "properties": { "algorithm": { "const": "RSA" } }, "required": ["algorithm"] },
              "then": { "properties": { "size": { "enum": [2048, 4096] } } }
            }
          ]
        },
        "duration":    { "type": "string", "pattern": "^[0-9]+h$" },
        "renewBefore": { "type": "string", "pattern": "^[0-9]+h$" }
      }
    },
    "rotation": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "alertExpiringHours":       { "type": "integer", "minimum": 24, "maximum": 720 },
        "alertStuckHours":          { "type": "integer", "minimum": 720, "maximum": 8760 },
        "alertHandshakeFailurePct": { "type": "number",  "minimum": 0.01, "maximum": 100.0 }
      }
    }
  }
}
```

### 13.3 tls-issuer.yaml extension (ClusterIssuer)

```yaml
{{- if and .Values.enabled .Values.tls.enabled .Values.tls.certManager.enabled }}
apiVersion: cert-manager.io/v1
kind: {{ ternary "ClusterIssuer" "Issuer" .Values.tls.certManager.clusterIssuer }}
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-issuer
  ...
```

A ClusterIssuer is a cluster-scoped resource (cert-manager CRD). It's used
when EFE-meta-swarm deploys across multiple namespaces and a shared CA is
preferred. The schema gates this opt-in.

### 13.4 tls-certificate.yaml extension (extra SANs + privateKey config)

```yaml
spec:
  ...
  privateKey:
    algorithm: {{ .Values.tls.certManager.privateKey.algorithm }}
    size:      {{ .Values.tls.certManager.privateKey.size }}
    rotationPolicy: {{ .Values.tls.certManager.privateKey.rotationPolicy }}
  duration:    {{ .Values.tls.certManager.duration | default "720h" }}
  renewBefore: {{ .Values.tls.certManager.renewBefore | default "168h" }}
  dnsNames:
    - {{ include "efe-meta-swarm.controllerName" . }}.{{ .Release.Namespace }}.svc
    - {{ include "efe-meta-swarm.controllerName" . }}.{{ .Release.Namespace }}.svc.cluster.local
    {{- range $san := .Values.tls.additionalSans }}
    - {{ $san | quote }}
    {{- end }}
```

---

## 14. Binary-side : `crates/efe-meta-swarm-server/src/tls/` module

### 14.1 Module surface (`tls/mod.rs`)

```rust
//! TLS / mTLS support for the EFE meta-swarm controller and participants.
//!
//! See `docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md` for the full design.

mod config;
mod reader;
mod watcher;
mod reload;
mod metrics;
mod error;

pub use config::{TlsServerConfig, TlsClientConfig, ProtocolVersion};
pub use reload::{TlsReloader, TlsReloadEvent};
pub use error::TlsError;
pub use metrics::TlsMetrics;
```

### 14.2 Public types

```rust
// tls/config.rs

#[derive(Debug, Clone)]
pub struct TlsServerConfig {
    pub identity:        rustls::sign::CertifiedKey,
    pub client_ca_roots: Option<rustls::RootCertStore>,
    pub min_protocol:    ProtocolVersion,
    pub require_mutual:  bool,
    pub allowed_sans:    Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TlsClientConfig {
    pub identity:        rustls::sign::CertifiedKey,
    pub server_ca_roots: rustls::RootCertStore,
    pub min_protocol:    ProtocolVersion,
    pub expected_san:    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolVersion {
    Tls12,
    Tls13,
}
```

### 14.3 Cert reader (`tls/reader.rs`)

```rust
pub fn load_certs(path: &Path) -> Result<Vec<rustls::Certificate>, TlsError> {
    let pem = fs::read(path).map_err(TlsError::Io)?;
    let mut reader = io::BufReader::new(pem.as_slice());
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| TlsError::PemParse(e.to_string()))?;
    if certs.is_empty() {
        return Err(TlsError::EmptyCertFile(path.to_path_buf()));
    }
    Ok(certs.into_iter().map(rustls::Certificate).collect())
}

pub fn load_private_key(path: &Path) -> Result<rustls::PrivateKey, TlsError> {
    let pem = fs::read(path).map_err(TlsError::Io)?;
    let mut reader = io::BufReader::new(pem.as_slice());
    // Try PKCS8 first ; fall back to RSA ; fail on unknown.
    if let Some(key) = rustls_pemfile::pkcs8_private_keys(&mut reader).next() {
        return Ok(rustls::PrivateKey(key?.secret_pkcs8_der().to_vec()));
    }
    let mut reader = io::BufReader::new(pem.as_slice());
    if let Some(key) = rustls_pemfile::rsa_private_keys(&mut reader).next() {
        return Ok(rustls::PrivateKey(key?.secret_pkcs1_der().to_vec()));
    }
    Err(TlsError::UnsupportedKeyFormat(path.to_path_buf()))
}
```

### 14.4 Error type

```rust
#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("I/O error : {0}")]
    Io(#[from] std::io::Error),
    #[error("PEM parse error : {0}")]
    PemParse(String),
    #[error("Cert file is empty : {0}")]
    EmptyCertFile(PathBuf),
    #[error("Unsupported key format : {0}")]
    UnsupportedKeyFormat(PathBuf),
    #[error("rustls config error : {0}")]
    RustlsConfig(String),
    #[error("SAN mismatch : expected {expected}, got {actual:?}")]
    SanMismatch { expected: String, actual: Vec<String> },
    #[error("Watcher error : {0}")]
    Watcher(String),
    #[error("Reload timeout")]
    ReloadTimeout,
}
```

---

## 15. tonic-tls integration — server side (controller)

### 15.1 Server config builder

```rust
// controller/server.rs — MODIFIED

use efe_meta_swarm_server::tls::{TlsServerConfig, ProtocolVersion};
use tonic::transport::{Server, ServerTlsConfig, Identity, Certificate as TonicCert};

async fn build_server(tls_cfg: Option<TlsServerConfig>) -> Result<Server, Error> {
    let mut server = Server::builder()
        .timeout(Duration::from_secs(30))
        .tcp_keepalive(Some(Duration::from_secs(30)));

    if let Some(cfg) = tls_cfg {
        let identity = Identity::from_pem(&cfg.cert_chain_pem, &cfg.key_pem);
        let mut tls = ServerTlsConfig::new().identity(identity);
        if cfg.require_mutual {
            let ca = TonicCert::from_pem(&cfg.client_ca_pem);
            tls = tls
                .client_ca_root(ca)
                .client_auth_optional(false);  // strict
        }
        // protocol version override (rustls)
        let rustls_cfg = build_rustls_server_config(&cfg)?;
        tls = tls.rustls_server_config(rustls_cfg);
        server = server.tls_config(tls)?;
    }

    Ok(server)
}
```

### 15.2 rustls server-config builder (protocol + cipher policy)

```rust
fn build_rustls_server_config(cfg: &TlsServerConfig) -> Result<rustls::ServerConfig, TlsError> {
    let protocols: &[&rustls::SupportedProtocolVersion] = match cfg.min_protocol {
        ProtocolVersion::Tls13 => &[&rustls::version::TLS13],
        ProtocolVersion::Tls12 => &[&rustls::version::TLS12, &rustls::version::TLS13],
    };

    let builder = rustls::ServerConfig::builder()
        .with_safe_default_cipher_suites()
        .with_safe_default_kx_groups()
        .with_protocol_versions(protocols)
        .map_err(|e| TlsError::RustlsConfig(e.to_string()))?;

    let verifier: Arc<dyn rustls::server::ClientCertVerifier> = if cfg.require_mutual {
        Arc::new(StrictSanVerifier::new(
            cfg.client_ca_roots.as_ref().unwrap().clone(),
            cfg.allowed_sans.clone(),
        ))
    } else {
        Arc::new(rustls::server::NoClientAuth)
    };

    let server_cfg = builder
        .with_client_cert_verifier(verifier)
        .with_single_cert(cfg.identity.cert.clone(), cfg.identity.key.clone())
        .map_err(|e| TlsError::RustlsConfig(e.to_string()))?;

    Ok(server_cfg)
}
```

### 15.3 Strict-SAN client cert verifier

```rust
struct StrictSanVerifier {
    roots: rustls::RootCertStore,
    allowed_sans: Vec<String>,
}

impl rustls::server::ClientCertVerifier for StrictSanVerifier {
    fn client_auth_root_subjects(&self) -> &[rustls::DistinguishedName] {
        self.roots.subjects()
    }

    fn verify_client_cert(
        &self,
        end_entity: &rustls::Certificate,
        intermediates: &[rustls::Certificate],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::server::ClientCertVerified, rustls::Error> {
        // 1. Chain verification (delegated to webpki)
        let cert = webpki::EndEntityCert::try_from(end_entity.as_ref())?;
        cert.verify_is_valid_tls_client_cert(
            SIGNATURE_ALGORITHMS,
            &webpki::TlsClientTrustAnchors(&self.trust_anchors()),
            intermediates,
            now,
        )?;
        // 2. SAN constraint
        let sans = extract_sans(&cert)?;
        if !sans.iter().any(|s| self.allowed_sans.iter().any(|allowed| s == allowed)) {
            return Err(rustls::Error::General(format!(
                "client cert SAN mismatch : sans={sans:?} allowed={:?}",
                self.allowed_sans
            )));
        }
        Ok(rustls::server::ClientCertVerified::assertion())
    }
}
```

---

## 16. tonic-tls integration — client side (controller→participant)

### 16.1 Channel builder

```rust
// controller/client.rs — MODIFIED

use tonic::transport::{Channel, ClientTlsConfig, Identity, Certificate as TonicCert};

pub async fn build_channel(
    endpoint: &str,
    tls_cfg: Option<TlsClientConfig>,
) -> Result<Channel, Error> {
    let mut builder = Channel::from_shared(endpoint.to_string())?
        .timeout(Duration::from_secs(30))
        .keep_alive_timeout(Duration::from_secs(10));

    if let Some(cfg) = tls_cfg {
        let identity = Identity::from_pem(&cfg.cert_chain_pem, &cfg.key_pem);
        let ca = TonicCert::from_pem(&cfg.server_ca_pem);
        let tls = ClientTlsConfig::new()
            .domain_name(cfg.expected_san.clone())   // ← SAN must match this
            .ca_certificate(ca)
            .identity(identity);
        builder = builder.tls_config(tls)?;
    }

    Ok(builder.connect().await?)
}
```

### 16.2 Per-participant channel pool

The controller maintains a pool of mTLS channels — one per participant. The
TLS reload coordinator (§ 19) refreshes each channel when the cert changes,
re-handshaking against the participant.

```rust
// controller/state.rs — MODIFIED

pub struct ControllerChannelPool {
    channels: DashMap<ParticipantId, Channel>,
    tls_reloader: Arc<TlsReloader>,
}

impl ControllerChannelPool {
    pub async fn ensure_channels(&self, participants: &[ParticipantConfig]) -> Result<(), Error> {
        let tls_cfg = self.tls_reloader.current_client_config();
        for p in participants {
            if !self.channels.contains_key(&p.id) {
                let ch = build_channel(&p.endpoint, tls_cfg.clone()).await?;
                self.channels.insert(p.id.clone(), ch);
            }
        }
        Ok(())
    }

    pub async fn refresh_all(&self) -> Result<(), Error> {
        // Called by TlsReloader on cert change.
        let new_tls = self.tls_reloader.current_client_config();
        let participants: Vec<_> = self.channels.iter().map(|e| e.key().clone()).collect();
        for pid in participants {
            // Build new channel, replace atomically.
            let endpoint = self.endpoint_for(&pid)?;
            let new_ch = build_channel(&endpoint, new_tls.clone()).await?;
            self.channels.insert(pid, new_ch);
        }
        Ok(())
    }
}
```

---

## 17. tonic-tls integration — server side (participant)

Identical structure to § 15 (controller server), but mounted at the
participant binary's bin/participant.rs entry point. Each participant has its
own Identity (its own cert + key) and verifies the controller's cert against
the same CA chain.

The verifier is identical (`StrictSanVerifier`) ; the `allowed_sans` for a
participant is the controller's DNS name. So the participant only accepts
mTLS connections from the controller (not from another participant, in the
Phase-2 single-controller topology).

This is an additional defense : even if the controller's cert is stolen, an
attacker would need to also be on the controller's IP and pass network policy
checks (PR #14 NetworkPolicy already enforces this).

---

## 18. Inotify-based cert file watcher

### 18.1 Implementation

```rust
// tls/watcher.rs

use notify::{Watcher, RecursiveMode, Event};

pub struct CertWatcher {
    tx: mpsc::Sender<CertChangeEvent>,
    _watcher: notify::RecommendedWatcher,
}

#[derive(Debug, Clone)]
pub struct CertChangeEvent {
    pub path: PathBuf,
    pub kind: notify::EventKind,
    pub at:   Instant,
}

impl CertWatcher {
    pub fn new(paths: Vec<PathBuf>) -> Result<(Self, mpsc::Receiver<CertChangeEvent>), TlsError> {
        let (tx, rx) = mpsc::channel(64);
        let tx_clone = tx.clone();

        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                if matches!(event.kind, notify::EventKind::Modify(_) | notify::EventKind::Create(_)) {
                    for p in &event.paths {
                        let _ = tx_clone.blocking_send(CertChangeEvent {
                            path: p.clone(),
                            kind: event.kind,
                            at:   Instant::now(),
                        });
                    }
                }
            }
        }).map_err(|e| TlsError::Watcher(e.to_string()))?;

        for path in paths {
            watcher.watch(&path, RecursiveMode::NonRecursive)
                .map_err(|e| TlsError::Watcher(e.to_string()))?;
        }

        Ok((Self { tx, _watcher: watcher }, rx))
    }
}
```

### 18.2 Debouncing

Kubernetes Secret mounts are tricky : a Secret update typically triggers
multiple inotify events (a few RENAME + CREATE + WRITE within ~ 100 ms). The
watcher debounces : a burst of events on the same file within 500 ms triggers
only ONE reload.

```rust
async fn debounce_loop(
    mut rx: mpsc::Receiver<CertChangeEvent>,
    reload_tx: mpsc::Sender<ReloadRequest>,
) {
    let mut last_seen = HashMap::<PathBuf, Instant>::new();
    while let Some(event) = rx.recv().await {
        let now = Instant::now();
        let last = last_seen.get(&event.path).copied().unwrap_or(now - Duration::from_secs(60));
        if now.duration_since(last) < Duration::from_millis(500) {
            continue;  // debounce
        }
        last_seen.insert(event.path.clone(), now);
        let _ = reload_tx.send(ReloadRequest::CertChanged(event.path)).await;
    }
}
```

### 18.3 Non-Linux fallback : 5s polling

```rust
#[cfg(not(target_os = "linux"))]
pub fn poll_watcher(paths: Vec<PathBuf>) -> mpsc::Receiver<CertChangeEvent> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        let mut last_mtimes = HashMap::<PathBuf, SystemTime>::new();
        loop {
            for p in &paths {
                if let Ok(meta) = fs::metadata(p) {
                    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    let prev = last_mtimes.get(p).copied();
                    if Some(mtime) != prev {
                        let _ = tx.send(CertChangeEvent {
                            path: p.clone(),
                            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
                            at:   Instant::now(),
                        }).await;
                        last_mtimes.insert(p.clone(), mtime);
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    rx
}
```

---

## 19. Hot reload + connection drain protocol

### 19.1 Reload coordinator

```rust
// tls/reload.rs

pub struct TlsReloader {
    current_server: ArcSwap<TlsServerConfig>,
    current_client: ArcSwap<TlsClientConfig>,
    drain_grace:    Duration,
    metrics:        Arc<TlsMetrics>,
}

impl TlsReloader {
    pub fn current_server_config(&self) -> Arc<TlsServerConfig> {
        self.current_server.load_full()
    }

    pub fn current_client_config(&self) -> Arc<TlsClientConfig> {
        self.current_client.load_full()
    }

    pub async fn handle_cert_change(&self, path: PathBuf) -> Result<(), TlsError> {
        let span = tracing::info_span!("tls.reload", path = ?path);
        let _enter = span.enter();
        let start = Instant::now();

        // 1. Re-read cert + key from disk.
        let new_server = self.read_server_config().await?;
        let new_client = self.read_client_config().await?;

        // 2. Atomic swap (lock-free for readers).
        self.current_server.store(Arc::new(new_server));
        self.current_client.store(Arc::new(new_client));

        // 3. Trigger graceful drain on existing connections.
        // (tonic Server keeps connections open until they idle ; new
        //  connections will use the new config automatically because the
        //  server_config closure reads from current_server)
        // No active connection-kill needed ; the drain happens passively.

        // 4. Metrics + audit.
        self.metrics.record_reload(start.elapsed(), ReloadResult::Ok);

        tracing::info!("cert reload completed in {:?}", start.elapsed());
        Ok(())
    }
}
```

### 19.2 Why no explicit drain

rustls + tonic's design : the server's TLS config is read at connection
acceptance time. New connections after the swap use the new config. In-flight
connections (already established with the old cert) continue with the old
session keys — they were derived during handshake and don't depend on the
current config.

So the "drain" is implicit : in-flight RPCs complete on their existing
connections ; new RPCs on those connections get HTTP/2 stream multiplex on
the existing TLS session (still using the old cert's session). New connections
established post-reload use the new cert.

The 30 s grace period (AD10) is the max time we expect existing connections
to remain : if a connection idles > 30 s, it's typically reaped by the
keep-alive timer, then re-established with the new cert.

### 19.3 Verifying 0-downtime

Tested by the chaos bench (`integration_tls_rotation.rs`) :

  1. Establish 100 long-lived gRPC streams.
  2. Force cert rotation.
  3. Verify 0 streams broken.
  4. Verify all new streams use the new cert (peek at handshake session ID).

---

## 20. TLS metrics — 4 new Prometheus series

```rust
// tls/metrics.rs

pub struct TlsMetrics {
    pub handshake_duration: HistogramVec,        // "efe_meta_swarm_tls_handshake_duration_seconds"
    pub handshake_total:    IntCounterVec,        // "efe_meta_swarm_tls_handshake_total"
    pub cert_expiry:        GaugeVec,             // "efe_meta_swarm_tls_cert_expiry_seconds"
    pub reload_total:       IntCounterVec,        // "efe_meta_swarm_tls_reload_total"
}
```

| Metric | Type | Labels | Description |
|---|---|---|---|
| `efe_meta_swarm_tls_handshake_duration_seconds` | Histogram | `result`, `protocol`, `role` | Handshake duration in seconds. `result` = ok/fail ; `protocol` = 1.2/1.3 ; `role` = server/client. Buckets : 1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s |
| `efe_meta_swarm_tls_handshake_total` | Counter | `result`, `failure_reason` | Total handshakes. `failure_reason` ∈ {expired_cert, san_mismatch, untrusted_ca, protocol_downgrade, other} |
| `efe_meta_swarm_tls_cert_expiry_seconds` | Gauge | `cert_name`, `subject_cn` | Unix-time seconds remaining until cert expiry |
| `efe_meta_swarm_tls_reload_total` | Counter | `result` | Total cert reloads (success or failure) |

### 20.1 Metric sampling : cert expiry

```rust
async fn cert_expiry_loop(metrics: Arc<TlsMetrics>, reloader: Arc<TlsReloader>) {
    loop {
        let cfg = reloader.current_server_config();
        for cert in cfg.identity.cert.iter() {
            if let Ok(parsed) = x509_parser::parse_x509_certificate(&cert.0) {
                let not_after = parsed.1.validity().not_after;
                let now_unix = SystemTime::now()
                    .duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
                let expiry_unix = not_after.timestamp();
                let secs_remaining = expiry_unix - now_unix;
                metrics.cert_expiry
                    .with_label_values(&[&parsed.1.subject().to_string(), "controller"])
                    .set(secs_remaining as f64);
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
```

---

## 21. PrometheusRule alerts — 3 new alerts

```yaml
# charts/efe-stack/charts/efe-meta-swarm/templates/tls-prometheusrule.yaml
{{- if and .Values.enabled .Values.alerts.enabled .Values.tls.enabled }}
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: {{ include "efe-meta-swarm.fullname" . }}-tls
  labels:
    {{- include "efe-meta-swarm.labels" . | nindent 4 }}
    prometheus: efe
spec:
  groups:
    - name: efe-meta-swarm.tls
      interval: 60s
      rules:

        - alert: MetaSwarmTlsCertExpiringSoon
          expr: |
            min(efe_meta_swarm_tls_cert_expiry_seconds) by (cert_name)
              < {{ .Values.tls.rotation.alertExpiringHours }} * 3600
          for: 5m
          labels:
            severity: warning
            component: meta-swarm
          annotations:
            summary: "TLS cert {{`{{ $labels.cert_name }}`}} expiring < {{ .Values.tls.rotation.alertExpiringHours }} h"
            description: |
              cert-manager should have rotated this cert 7 days before
              expiry (renewBefore: 168h). If this alert fires, cert-manager
              may be stuck.  Investigate :
              `kubectl describe certificate {{`{{ $labels.cert_name }}`}}`.
            runbook_url: "{{ .Values.alerts.runbookBaseUrl }}#tls-cert-expiring"

        - alert: MetaSwarmTlsHandshakeFailureRate
          expr: |
            sum(rate(efe_meta_swarm_tls_handshake_total{result="fail"}[5m])) by (role)
              / sum(rate(efe_meta_swarm_tls_handshake_total[5m])) by (role)
              > {{ .Values.tls.rotation.alertHandshakeFailurePct }} / 100
          for: 5m
          labels:
            severity: critical
            component: meta-swarm
          annotations:
            summary: "TLS handshake failure rate > {{ .Values.tls.rotation.alertHandshakeFailurePct }} % on {{`{{ $labels.role }}`}}"
            description: |
              Likely causes : cert chain mismatch, CA stale, SAN mismatch,
              protocol downgrade attempt. Investigate via the failure_reason
              label : `sum by (failure_reason) (efe_meta_swarm_tls_handshake_total{result="fail"})`.
            runbook_url: "{{ .Values.alerts.runbookBaseUrl }}#tls-handshake-failure"

        - alert: MetaSwarmTlsRotationStuck
          expr: |
            time() - max(efe_meta_swarm_tls_reload_at_seconds) > {{ .Values.tls.rotation.alertStuckHours }} * 3600
          for: 1h
          labels:
            severity: warning
            component: meta-swarm
          annotations:
            summary: "TLS cert rotation stuck (no reload in {{ .Values.tls.rotation.alertStuckHours }} h)"
            description: |
              cert-manager should rotate every 23 days (30d duration − 7d renewBefore).
              If we haven't seen a reload in 45 days, cert-manager is broken.
            runbook_url: "{{ .Values.alerts.runbookBaseUrl }}#tls-rotation-stuck"
{{- end }}
```

---

## 22. OpenTelemetry tracing — TLS handshake spans

```rust
// in tls/reload.rs and tonic interceptor

let span = tracing::info_span!(
    "tls.handshake",
    protocol = ?protocol_version,
    cipher = ?negotiated_cipher,
    role = ?role,
    peer_san = ?peer_san,
);
let _enter = span.enter();
// ... handshake ...
span.record("result", &result);
span.record("duration_ms", &elapsed.as_millis());
```

3 spans :

  - `tls.handshake` — every TLS handshake (server + client).
  - `tls.reload` — every cert-file reload event.
  - `tls.cert_check` — periodic cert-expiry sampling.

These wire into the existing OTel pipeline (PR #18 will add 8 more spans for
the controller / participant request paths).

---

## 23. Audit chain — TLS handshake events

Every handshake (success or failure) writes an audit event to the
audit_events table :

```sql
INSERT INTO audit_events (
    tenant_id, event_type, peer_subject, result, cipher_suite,
    protocol_version, hmac_chain_link, occurred_at
) VALUES (
    NULL,                                  -- tenant-less (infra-level event)
    'tls_handshake',
    'CN=tst-efe-meta-swarm-controller',    -- sanitized, no PII
    'ok',
    'TLS_AES_128_GCM_SHA256',
    'TLSv1_3',
    decode('...', 'hex'),
    now()
);
```

The HMAC chain link continues the per-tenant chain (or the infra-level
chain if `tenant_id IS NULL`). PR #16 adds the `tls_handshake` event type
to the existing audit chain — no schema migration needed (column types
unchanged).

---

## 24. Backward compatibility + migration path

### 24.1 Default state (post-PR #16)

  - `tls.enabled: false` (same as PR #14 default).
  - No Issuer, no Certificate, no metrics, no alerts, no hooks.
  - Binary works plain-text.
  - All PR #14 tests pass unchanged.

### 24.2 Operator opt-in

```bash
helm upgrade tst ./charts/efe-stack \
  --set efe-meta-swarm.tls.enabled=true \
  --set efe-meta-swarm.tls.mutual=true \
  --set efe-meta-swarm.tls.certManager.enabled=true
```

Result : cert-manager Issuer + 4 Certificates created ; wait-for-cert hook
runs ; controllers/participants come up with mTLS.

### 24.3 Rolling migration

Staged migration over 30 days :

  - Day 0 : staging cluster, `tls.enabled: true`.
  - Day 14 : production canary slice, `tls.enabled: true`.
  - Day 30 : production-prime, `tls.enabled: true`.

After 30 days of clean ops, default in chart bumps from `false` → `true` in
a future PR (not in PR #16 ; that's PR #16.next).

### 24.4 Downgrade path

If mTLS is breaking, `helm upgrade ... --set efe-meta-swarm.tls.enabled=false`.
Pods restart in plain-text mode. cert-manager Certificates persist (harmless).
A subsequent `helm uninstall` removes them.

---

## 25. mTLS opt-in vs server-side TLS only

### 25.1 Server-side TLS only (`tls.mutual: false`)

  - Server presents a cert.
  - Client validates server cert.
  - Server does NOT request a client cert.
  - Client connections from the helm-test probe Pod work (probe has no cert).
  - Less secure (no client authentication), but useful for early adoption.

### 25.2 Mutual TLS (`tls.mutual: true`)

  - Both server AND client present certs.
  - Both ends validate each other's certs.
  - Both ends do strict SAN validation.
  - The helm-test probe Pod CANNOT connect (it has no cert) — by design.
  - Most secure ; production target.

### 25.3 Recommendation

  - Dev / staging : `mutual: false` (quick iteration).
  - Production : `mutual: true` (full mTLS).

The chart's `values-prod.yaml` overlay sets `mutual: true` by default.

---

## 26. Schema additions (`values.schema.json`)

Already covered in § 13.2. Full schema diff in `values.schema.json` will
add ~ 80 LOC.

---

## 27. Helm hook ordering + weights

Refined hook weights (PR #14 used `-20`, `-19`, `-18`, `-10`, `10` ; PR #16
adds `-5` for the wait-for-cert hook) :

```text
weight  hook                                        sprint added
─────── ─────────────────────────────────────────── ─────────────
-20     hook-serviceaccount (pre-install,upgrade)   PR #14
-19     hook-role           (pre-install,upgrade)   PR #14
-18     hook-rolebinding    (pre-install,upgrade)   PR #14
-10     pre-upgrade-snapshot Job                    PR #14
 -5     tls-wait-for-cert Job (pre-install,upgrade) PR #16 ← NEW
 10     post-upgrade-sighup Job                     PR #14
```

The `-5` slot is BEFORE the snapshot Job (`-10`) and AFTER the hook SA / Role /
RoleBinding (`-20` / `-19` / `-18`). Logically : we wait for certs as soon as
the auth infrastructure is in place, before any other workload runs.

---

## 28. Operator-driven workflows : enable, rotate, disable

### 28.1 Enable mTLS (Day 0)

```bash
# 1. Verify cert-manager is installed.
kubectl get pods -n cert-manager
# Expected : cert-manager + cert-manager-cainjector + cert-manager-webhook all Running

# 2. (optional) Create the CA Secret if using `issuerType: ca`.
kubectl create secret tls my-org-root-ca \
  --cert=root-ca.crt \
  --key=root-ca.key \
  -n efe

# 3. Helm upgrade with mTLS enabled.
helm upgrade tst ./charts/efe-stack \
  -n efe \
  -f values-prod.yaml \
  --set efe-meta-swarm.tls.enabled=true \
  --set efe-meta-swarm.tls.mutual=true \
  --set efe-meta-swarm.tls.certManager.enabled=true \
  --set efe-meta-swarm.tls.certManager.issuerType=ca \
  --set efe-meta-swarm.tls.certManager.caSecretName=my-org-root-ca

# 4. Observe :
kubectl get certificates -n efe
# Expected : 4 Certificates Ready=True

# 5. Verify mTLS active :
kubectl logs -n efe deploy/tst-efe-meta-swarm-controller | grep "TLS active"
# Expected : "TLS active : protocol=TLSv1_3 cipher=TLS_AES_128_GCM_SHA256"
```

### 28.2 Manual rotation (emergency)

```bash
# Force cert rotation by triggering renewal :
kubectl annotate certificate tst-efe-meta-swarm-controller-tls \
  cert-manager.io/issue-temporary-certificate="true" \
  --overwrite -n efe

# cert-manager will re-issue. The Secret is updated. The pod's inotify
# watcher catches it. No restart.

# Verify rotation completed :
kubectl get certificate tst-efe-meta-swarm-controller-tls -n efe \
  -o jsonpath='{.status.notAfter}'
```

### 28.3 Disable mTLS

```bash
helm upgrade tst ./charts/efe-stack -n efe \
  --set efe-meta-swarm.tls.enabled=false

# Pods restart in plain-text mode. Certificates remain (manual cleanup):
kubectl delete certificate -l app.kubernetes.io/part-of=efe -n efe
```

---

## 29. Concurrency / lifecycle invariants

  - The inotify watcher runs in a background `tokio::spawn` task.
  - The reload coordinator uses `ArcSwap` for lock-free reads.
  - Multiple concurrent handshakes against the controller don't block each
    other (rustls is lock-free for handshakes against the same ServerConfig).
  - The cert-expiry sampling loop runs at 60-second cadence on a single task.
  - Reload events are debounced at 500 ms (§ 18.2).
  - In-flight connections are NEVER force-killed (AD10).
  - K8s liveness probe MUST NOT depend on cert-reload status (AD12) ; the
    health check is decoupled from TLS state.

---

## 30. Threat-model coverage (T35-T45)

| Threat | Description | Mitigation |
|---|---|---|
| **T35** | Cert private key exposure via Secret read | RBAC : Secrets readable only by the controller + participant SAs. Audit chain logs every Secret access. |
| **T36** | Weak TLS cipher negotiated | TLS 1.3-only by default (AD6). Cipher list frozen at chart version (AD30). Schema rejects `minProtocolVersion < "1.3"` without explicit opt-in. |
| **T37** | Cert rotation drops connections | Inotify watcher + ArcSwap atomic swap (§ 19). 30 s grace period. Chaos bench validates 0 drops. |
| **T38** | CA bundle staleness — peer trusts an expired root | cert-manager handles root rotation. PrometheusRule alerts at 7 days remaining (§ 21). |
| **T39** | SAN validation bypass | Strict SAN verifier (§ 15.3). IP SANs forbidden (§ 10.3). Wildcard SANs not issued (§ 10.4). |
| **T40** | cert-manager outage during issuance | Wait-for-cert Helm hook times out at 5 min ; install fails (fail-closed, AD25). Operator investigates. |
| **T41** | Client cert spoofing | Mutual TLS (`tls.mutual: true`) + strict SAN ensures the controller only accepts client certs whose SAN matches the controller's `allowed_sans` list. Combined with NetworkPolicy from PR #14. |
| **T42** | Protocol downgrade attack | TLS 1.3-only by default. TLS 1.2 opt-in flagged in audit chain as a downgrade event. Cipher list excludes RSA-only. |
| **T43** | PII in handshake logs | Audit chain logs `peer_subject` (CN only), no other cert fields. Cert SANs may contain DNS names which are not PII per GDPR Art. 30. |
| **T44** | mTLS over loopback unnecessarily | Loopback connections (`127.0.0.1`, `::1`) bypass TLS (AD18). Reduces overhead. |
| **T45** | TLS session replay | TLS 1.3 forbids replay-able mechanisms by default. 0-RTT (early data) explicitly disabled in rustls config. |

---

## 31. Tests strategy — Rust + Helm

### 31.1 Rust unit tests

  - `tls/reader.rs` : cert + key parsing, error cases (empty, malformed, wrong format).
  - `tls/watcher.rs` : inotify events captured, debouncing works, fallback poll works.
  - `tls/reload.rs` : ArcSwap atomic swap, no torn reads.
  - `tls/config.rs` : protocol version selection, cipher list, mutual flag.
  - `tls/metrics.rs` : counter increments correctly.

### 31.2 Rust integration tests

  - `integration_tls_handshake.rs` : end-to-end mTLS handshake controller↔participant ;
    verify protocol = TLS 1.3, cipher = AES-128-GCM-SHA256, SAN match.
  - `integration_tls_rotation.rs` : run 100 long-lived streams ; rotate cert
    mid-stream ; verify 0 streams broken + new streams use new cert.
  - `integration_tls_mtls_strict.rs` : verify `mutual=true` rejects clients without certs ;
    `mutual=false` accepts.
  - `integration_tls_san_validation.rs` : verify cert with wrong SAN is rejected ;
    cert with correct SAN accepted.
  - `integration_tls_disabled.rs` : verify `tls.enabled=false` works plain-text (backward-compat).

### 31.3 Helm unit tests

`tests/tls_test.yaml` extensions :

  - `AC27 (PR #16) — Wait-for-cert hook renders when tls.certManager.enabled`
  - `AC28 (PR #16) — Wait-for-cert hook has pre-install/pre-upgrade weight -5`
  - `AC29 (PR #16) — ClusterIssuer rendered when tls.certManager.clusterIssuer=true`
  - `AC30 (PR #16) — PrivateKey algorithm/size validated via schema`
  - `AC31 (PR #16) — TLS PrometheusRule renders 3 alerts when alerts.enabled+tls.enabled`
  - `AC32 (PR #16) — Mutual TLS renders client_ca_root on server config`

### 31.4 Schema rejection tests

`tests/schema_rejection_test.yaml` extensions :

  - tls.minProtocolVersion="1.0" → rejected
  - tls.certManager.privateKey.algorithm="DSA" → rejected
  - tls.certManager.privateKey.size=1024 → rejected
  - tls.certManager.privateKey.algorithm="ECDSA" + size=2048 → rejected (cross-field rule)
  - tls.waitForCertTimeoutSeconds=10 → rejected (< 60)
  - tls.waitForCertTimeoutSeconds=3600 → rejected (> 1800)

### 31.5 Golden snapshot regen

After PR #16 lands, regenerate `tests/golden/kovant-prod.yaml` with TLS enabled.
Diff is expected (4 Certificates + Issuer + wait-for-cert Job + tls-prometheusrule
all render). Audit-pass pattern from PR #14 ensures the goldens are reviewed
manually before merge.

### 31.6 Bench surface (WS2)

  - `SLO-TLS-HANDSHAKE` : p99 < 50 ms on staging tier.
  - `SLO-TLS-ROTATION` : 0 dropped connections during cert rotation under 1 000 QPS.

Both added to the Honest Bench Framework catalog (Appendix D of bench arch doc).

---

## 32. Acceptance criteria (AC1–AC32)

| AC | Statement | Status |
|---|---|---|
| AC1 | `tls.enabled: false` default ; chart renders no TLS resources. Backward-compat with PR #14 unchanged. | gate |
| AC2 | `tls.enabled: true` + `tls.certManager.enabled: true` renders Issuer + 4 Certificates + wait-for-cert hook. | gate |
| AC3 | Controller binary loads `/etc/efe-meta-swarm/tls/tls.crt` + `tls.key` + `ca.crt` when `EFE_META_SWARM_TLS_REQUIRED=true`. | gate |
| AC4 | Controller binary configures tonic Server with `ServerTlsConfig` when TLS required. | gate |
| AC5 | Controller binary configures tonic Channel with `ClientTlsConfig` for each participant. | gate |
| AC6 | Participant binary configures tonic Server with `ServerTlsConfig`. | gate |
| AC7 | TLS 1.3 used by default ; cipher suite negotiated from rustls defaults. | gate |
| AC8 | `tls.minProtocolVersion: "1.2"` allows TLS 1.2 + 1.3 ; `"1.3"` allows 1.3 only. | gate |
| AC9 | `tls.mutual: true` configures strict client-cert verification ; `false` configures `NoClientAuth`. | gate |
| AC10 | Cert file watcher (inotify) detects cert change ≤ 500 ms ; reload coordinator swaps config ≤ 100 ms. | gate |
| AC11 | Cert rotation under 1 000 QPS load : **0 connections dropped**. | gate |
| AC12 | New connections post-rotation use new cert. | gate |
| AC13 | Cert with SAN not matching the expected DNS name : handshake rejected with `NotValidForName`. | gate |
| AC14 | Cert with IP-only SAN : rejected. | gate |
| AC15 | Wildcard SAN cert : NOT issued by the configured Issuer (schema gates). | gate |
| AC16 | Wait-for-cert Helm hook runs at weight `-5` (pre-install, pre-upgrade). | gate |
| AC17 | Wait-for-cert hook times out at 5 min if certs not Ready. Install fails. | gate |
| AC18 | Cert-expiry gauge `efe_meta_swarm_tls_cert_expiry_seconds` reported. | gate |
| AC19 | Handshake duration histogram `efe_meta_swarm_tls_handshake_duration_seconds` reported. | gate |
| AC20 | Handshake total counter `efe_meta_swarm_tls_handshake_total{result=...}` reported. | gate |
| AC21 | Cert rotation total counter `efe_meta_swarm_tls_reload_total{result=...}` reported. | gate |
| AC22 | 3 PrometheusRule alerts render when `alerts.enabled+tls.enabled` : expiring, handshake-failure, rotation-stuck. | gate |
| AC23 | OpenTelemetry spans `tls.handshake`, `tls.reload`, `tls.cert_check` emitted. | gate |
| AC24 | Audit chain logs every handshake (success + failure) with `event_type=tls_handshake`. | gate |
| AC25 | Loopback connections (`127.0.0.1`, `::1`) bypass TLS regardless of `tls.enabled`. | gate |
| AC26 | TLS 1.3 forbids early-data / 0-RTT replay. | gate |
| AC27 | `tls.certManager.clusterIssuer: true` renders ClusterIssuer instead of Issuer. | gate |
| AC28 | Private-key algorithm + size in Certificate matches `tls.certManager.privateKey.*`. | gate |
| AC29 | Schema rejects invalid algorithm/size cross-product (ECDSA + 2048, RSA + 256, etc.). | gate |
| AC30 | Schema rejects `tls.minProtocolVersion: "1.0"` / `"1.1"`. | gate |
| AC31 | Hot-reload event records the new cert's expiry in the reload total counter labels. | gate |
| AC32 | Honest Bench Framework SLO-TLS-HANDSHAKE p99 < 50 ms on staging. | gate |

---

## 33. Risks + mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| cert-manager not installed when operator runs upgrade | Medium | High | Wait-for-cert hook times out + clear error message ; doc pre-flight checklist |
| Custom CA Secret missing | Medium | High | cert-manager Issuer condition fails ; wait-for-cert hook surfaces it |
| ARM64 + ECDSA-P256 performance | Low | Low | rustls SIMD-accelerated on aarch64 ; bench in WS2 |
| Inotify not available (e.g. user-mode k8s) | Low | Low | 5s polling fallback (§ 18.3) |
| Cert rotation while controller iterates | Medium | Medium | Drain protocol § 19 ; verified by chaos bench |
| TLS 1.2 opt-down breaks audit trail | Low | Low | Audit chain records protocol_version ; flagged as downgrade event in compliance reports |
| Performance regression from mTLS handshake | Low | Medium | Handshake p99 SLO ; alerted via PrometheusRule |
| cert-manager API changes between versions | Medium | Medium | Compatibility matrix § Appendix E ; pinned to v1.13+ |
| Helm hook RBAC missing `certificates` resource | Low | Medium | Hook Role explicitly grants `cert-manager.io/certificates : get,list` |
| Cipher suite policy customer-customized | Low | Medium | Chart version bump required for any cipher change (AD30) |

---

## 34. Rollback strategy

  1. Set `tls.enabled: false` in values + `helm upgrade`.
  2. Pods restart in plain-text mode.
  3. Existing Certificate resources remain (harmless, not consumed).
  4. Eventually remove via `kubectl delete certificate -l app.kubernetes.io/part-of=efe`.

If a partial failure (e.g. mTLS works on controller but not on participants) :

  1. Identify failing pod via PrometheusRule alert.
  2. Re-run wait-for-cert hook manually : `kubectl create job --from=cronjob/...` (or
     re-trigger via `helm upgrade --recreate-pods`).
  3. If still failing, set `tls.mutual: false` (server-side only — less strict)
     while investigating.

---

## 35. Sequence diagrams

### 35.1 Initial install with mTLS

```text
Operator         helm          cert-manager        wait-cert hook    controller     participant
   │             │              │                  │                 │              │
   ├ helm upgrade│              │                  │                 │              │
   │  --set      │              │                  │                 │              │
   │  tls.enabled│              │                  │                 │              │
   │             ├ pre-install  │                  │                 │              │
   │             │  hooks       │                  │                 │              │
   │             │  - hook SA   │                  │                 │              │
   │             │    Role, RB  │                  │                 │              │
   │             │  - snapshot  │                  │                 │              │
   │             │              │                  │                 │              │
   │             ├ create Issuer───>               │                 │              │
   │             ├ create 4 Certs──>               │                 │              │
   │             │              ├ issue certs      │                 │              │
   │             │              │ (~ 5-10 s)       │                 │              │
   │             │              ├ write Secrets    │                 │              │
   │             │              │                  │                 │              │
   │             ├ run wait-cert hook (-5)───>     │                 │              │
   │             │              │                  ├ poll Certs Ready│              │
   │             │              │                  │ (every 5s, 5min)│              │
   │             │              │  Ready=true────> │                 │              │
   │             │              │                  ├ exit 0          │              │
   │             │              │                  │                 │              │
   │             ├ create Deployments (controller + participants)    │              │
   │             │              │                  │                 ├ start        │
   │             │              │                  │                 ├ load TLS     │
   │             │              │                  │                 │  certs       │
   │             │              │                  │                 ├ configure    │
   │             │              │                  │                 │  tonic mTLS  │
   │             │              │                  │                 │              ├ start
   │             │              │                  │                 │              ├ load TLS
   │             │              │                  │                 │              ├ tonic mTLS
   │             │              │                  │                 │              │
   │             │              │                  │                 ├ mTLS connect─>
   │             │              │                  │                 │  handshake   │
   │             │              │                  │                 │<─ ok ────────│
   │             │              │                  │                 │              │
   │             │              │                  │                 ├ first        │
   │             │              │                  │                 │  Iterate RPC─>
   │             │              │                  │                 │              │
   │<── Helm OK ─│              │                  │                 │              │
```

### 35.2 Cert rotation (Day 23)

```text
cert-manager        Secret           inotify watcher    reload coord     controller   peer
     │              │                │                  │                │            │
     │ (Day 23, 23d into 30d duration)                  │                │            │
     ├ generate new │                │                  │                │            │
     │  key + CSR   │                │                  │                │            │
     ├ sign cert    │                │                  │                │            │
     ├ write Secret─>                │                  │                │            │
     │              │ (mount path    │                  │                │            │
     │              │  updated)      │                  │                │            │
     │              │                ├ event captured   │                │            │
     │              │                ├ debounce 500ms   │                │            │
     │              │                ├ trigger reload──>│                │            │
     │              │                │                  ├ re-read certs  │            │
     │              │                │                  ├ build new      │            │
     │              │                │                  │  rustls config │            │
     │              │                │                  ├ ArcSwap atomic │            │
     │              │                │                  │  swap          │            │
     │              │                │                  │                │            │
     │              │                │                  │ (existing connections continue
     │              │                │                  │  on old TLS session — keys
     │              │                │                  │  derived during handshake)
     │              │                │                  │                │            │
     │              │                │                  │ (new connections use new cert)
     │              │                │                  │                ├ new conn──>│
     │              │                │                  │                │  handshake │
     │              │                │                  │                │  uses new  │
     │              │                │                  │                │  cert      │
     │              │                │                  │                │            │
     │              │                │                  ├ emit reload_total++         │
     │              │                │                  ├ emit OTel span │            │
     │              │                │                  │  "tls.reload"  │            │
     │              │                │                  ├ audit event    │            │
```

---

## 36. Reviewer checklist

  - [ ] All 32 ACs have corresponding code or test stanzas.
  - [ ] All 11 threats (T35-T45) have named mitigations.
  - [ ] All 30 ADs have corresponding implementation or schema entries.
  - [ ] `cargo check --workspace --all-features` exits 0.
  - [ ] `cargo test -p efe-meta-swarm-server --release` exits 0.
  - [ ] `helm lint charts/efe-stack/charts/efe-meta-swarm --strict` exits 0.
  - [ ] `helm unittest` adds ≥ 6 new assertions (AC27-AC32 covered).
  - [ ] Schema rejection tests pass for invalid algorithm/size combos.
  - [ ] Golden snapshot regenerated for kovant-prod with TLS enabled.
  - [ ] Chart version bumped Chart.yaml.version 0.2.0 → 0.3.0 + appVersion 0.3.0 → 0.4.0.
  - [ ] CI master gate green (clippy + fmt + tests).
  - [ ] PR description references this architecture doc.
  - [ ] Chaos rotation bench passes (0 dropped connections under 1000 QPS).
  - [ ] OpenTelemetry spans visible in dev Jaeger.
  - [ ] Audit chain entries recorded for each handshake.
  - [ ] Honest Bench Framework SLO-TLS-HANDSHAKE measured.

---

## 37. Appendix A — TLS module Rust API surface

```rust
// pub re-exports from src/tls/mod.rs

pub use config::{
    TlsServerConfig,
    TlsClientConfig,
    ProtocolVersion,
    build_rustls_server_config,
    build_rustls_client_config,
};

pub use reader::{
    load_certs,
    load_private_key,
    load_ca_bundle,
    parse_cert_expiry,
};

pub use watcher::{
    CertWatcher,
    CertChangeEvent,
};

pub use reload::{
    TlsReloader,
    TlsReloadEvent,
    ReloadResult,
};

pub use metrics::{
    TlsMetrics,
};

pub use error::{
    TlsError,
};
```

---

## 38. Appendix B — example operator runbook (manual cert rotation)

In `docs/runbooks/tls-manual-rotation.md` (NEW) :

```markdown
# Manual TLS cert rotation

When : cert-manager has issued a suspicious cert (e.g. key exposure incident).

Steps :

1. Identify the affected Secret :
   kubectl get secret -l cert-manager.io/certificate-name=tst-efe-meta-swarm-controller-tls -n efe

2. Trigger cert-manager renewal :
   kubectl annotate certificate tst-efe-meta-swarm-controller-tls \
     cert-manager.io/issue-temporary-certificate=true \
     --overwrite -n efe

3. Watch the new cert get issued :
   kubectl get certificate -w -n efe

4. Verify the controller reloaded :
   kubectl logs deploy/tst-efe-meta-swarm-controller -n efe --tail=20 \
     | grep "tls.reload"

5. Audit chain entry :
   kubectl exec deploy/efe-server -n efe -- \
     psql -c "SELECT * FROM audit_events
              WHERE event_type='tls_reload'
              ORDER BY occurred_at DESC LIMIT 5;"

Expected : the new cert has an expiry 30 days in the future, and the
controller logged a `tls.reload` event ≤ 5 s after the Secret was updated.
```

---

## 39. Appendix C — SDK client mTLS guidance

The 4 SDKs do NOT need to be modified by PR #16. The mTLS path is internal
to EFE (controller↔participant). SDK clients connect to the gateway
(`efe-server`), which terminates TLS before fanning to meta-swarm.

If a customer wants end-to-end mTLS from SDK to meta-swarm controller :

  - That's an M27 edge concern. Out of scope for PR #16.
  - The SDK's TLS support (1.2+ in Python, 1.3 in Go, etc.) is already
    documented in each SDK's README.

---

## 40. Appendix D — open architectural questions resolved here

Per `EFE_MASTER_ROADMAP_2026_H2_TO_2027_H1.md` Appendix E :

  - **Q-01** : TLS cipher policy — TLS 1.3 only by default (resolved AD6).
  - **Q-02** : cert-manager Issuer kind — self-signed for staging, CA for prod, ClusterIssuer optional (resolved AD14).

Remaining open (deferred to PR #16.1 follow-up) :

  - HashiCorp Vault integration (cert-manager Vault Issuer wiring).
  - HSM-backed keys (Phase 3).
  - OCSP / CRL distribution (Phase 3).

---

## 41. Appendix E — cert-manager version compatibility matrix

| cert-manager version | supported by PR #16 ? | notes |
|---|---|---|
| < v1.10 | ❌ | Missing `Certificate.spec.privateKey.rotationPolicy: Always` |
| v1.10 - v1.12 | ⚠️ legacy | Works but no `additionalOutputFormats` ; deprecation warning |
| v1.13 | ✅ supported | Minimum supported. Tested. |
| v1.14 | ✅ supported | Tested. |
| v1.15+ | ✅ supported | Tested via CI matrix |

The chart's `Chart.yaml` declares a soft dependency :

```yaml
annotations:
  cert-manager.io/minimum-version: "v1.13.0"
```

The wait-for-cert hook script logs the detected cert-manager version :

```bash
kubectl get deployment cert-manager -n cert-manager \
  -o jsonpath='{.spec.template.spec.containers[0].image}'
```

---

## 42. Appendix F — cipher suite policy details

### TLS 1.3 (default)

```
TLS_AES_128_GCM_SHA256              (rustls priority 1)
TLS_AES_256_GCM_SHA384              (rustls priority 2)
TLS_CHACHA20_POLY1305_SHA256        (rustls priority 3)
```

### TLS 1.2 (opt-in via `tls.minProtocolVersion: "1.2"`)

In addition to the TLS 1.3 list :

```
TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256
TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384
TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256
```

NO RSA-only suites enabled even in TLS 1.2 mode.

### Frozen at chart version

Per AD30, adding or removing a cipher requires a minor chart version bump.
The current set is the rustls 0.21+ default-safe list.

---

## 43. Appendix G — SLSA-L3 attestation of cert provenance

For SLSA-L3 compliance (chart + binary release artifacts) :

  - cert-manager Certificate resource has a `cert-manager.io/certificate-provenance`
    annotation pointing at the bundle SBOM (CycloneDX).
  - The Helm chart's release SBOM lists the cert-manager version + Issuer kind.
  - The binary's TLS config is part of the Helm-rendered manifest, which is
    signed via cosign at release time.
  - An auditor can trace : binary→Helm manifest→cert-manager version→cert
    provenance.

---

## End of architecture document — PR #16

Architecture frozen. 10-commit code plan in § 3. Migration path § 24. Coexists
with PR #14 stubs by extending them — no breaking changes to existing operators.
