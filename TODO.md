# MemoBuild: Real-World Containerized Production-Grade Distributed Roadmap
**Baseline:** v0.4.0 (March 2026) → **Target:** v1.0.0 GA (Q4 2026)
**Scope:** Concrete engineering plan based on deep source analysis — not aspirational claims.
## Deep Analysis: Actual State vs. Claimed State
### What Is Real
* **Core DAG engine** (`src/core.rs`, `src/graph.rs`): solid BLAKE3-DAG incremental builds.
* **OCI exporter** (`src/oci/`): functional push/pull via Distribution Spec.
* **HTTP remote cache** (`src/remote_cache.rs`, `src/server/`): axum-based, works for single-region teams.
* **Consistent hashing ring** (`src/cache_cluster.rs`, 383 lines): multi-master replication protocol via HTTP/JSON — no TLS, no auth.
* **PostgreSQL store** (`src/scalable_db.rs`, 425 lines): deadpool connection pool, read replicas — functional but no migrations tooling.
* **Auto-scaler** (`src/auto_scaling.rs`, 415 lines): linear regression prediction + K8s HPA patch — no admission webhook, no PDB.
### Critical Gaps Not in Existing Roadmap
* No production Dockerfile for the MemoBuild server binary itself (the existing `Dockerfile` builds a *sample Node.js app*, not MemoBuild).
* No Docker Compose full-stack (MemoBuild cluster + PostgreSQL + Redis + Prometheus + Grafana).
* No Helm chart directory exists anywhere in the repo (only referenced in docs).
* `DistributedCache` layer methods (`has_layer`, `get_layer`, `put_layer`) delegate to local cache only — cluster replication not applied to OCI layers.
* No blob object-storage backend (S3/GCS/R2) — blobs stored on local disk, incompatible with StatefulSet-free scaling.
* No Prometheus `/metrics` endpoint — no scrape target for any monitoring stack.
* No mTLS — cluster nodes communicate in plaintext HTTP.
* No API authentication — any client can read/write any cache artifact.
* No SLSA provenance or Cosign signing — only referenced in docs.
* No Kubernetes Operator or CRDs — workloads require manual YAML authoring.
* No `NetworkPolicy`, `PodDisruptionBudget`, `PriorityClass` — not HA-safe.
* No multi-arch OCI image build (arm64/amd64) in CI.
* No automated garbage collection — manual `/gc` HTTP call only.

***
## Phase 0 — Production Containerization (v0.4.1) — 2 weeks
**Goal:** Make MemoBuild itself a production-grade OCI artifact before any distributed work proceeds.
### 0.1 Multi-Stage Server Dockerfile
Create `Dockerfile.server` with three stages:
* **Stage 1 `builder`:** `rust:1.82-slim-bookworm` — runs `cargo build --release --locked`.
* **Stage 2 `ca-certs`:** `debian:bookworm-slim` — extracts `/etc/ssl/certs` only.
* **Stage 3 `runtime`:** `gcr.io/distroless/cc-debian12:nonroot` — copies binary + CA certs. Final image ≈ 12 MB.
Build args: `TARGETARCH` for cross-compilation (`cross` crate).
Labels: OCI `org.opencontainers.image.*` annotations, SBOM pointer label.
### 0.2 Docker Compose Full-Stack (`docker-compose.prod.yml`)
Services required for a realistic local distributed environment:
* `memobuild-node1/2/3`: cluster nodes on ports 9090/9091/9092, peer-linked.
* `postgres`: `postgres:16-alpine`, init SQL from `scripts/db/init.sql`, health-checked.
* `redis`: `redis:7-alpine` with `maxmemory-policy allkeys-lru`, for L1 distributed cache.
* `prometheus`: scrapes `/metrics` from all cluster nodes.
* `grafana`: pre-provisioned dashboards via `grafana/provisioning/`.
* `jaeger`: all-in-one for distributed tracing (`OTEL_EXPORTER_OTLP_ENDPOINT`).
* `minio`: S3-compatible blob backend for artifact storage in dev.
Environment variables via `.env.example` with documented entries.
### 0.3 Multi-Arch CI Image Build
Add `.github/workflows/docker.yml`:
* `docker/setup-buildx-action` with QEMU for `linux/amd64,linux/arm64`.
* Build + push to `ghcr.io/nrelab/memobuild:{version,latest,sha}` on tag.
* Cosign keyless signing (`sigstore/cosign-installer`) for each arch-specific digest.
* SBOM generation via `anchore/sbom-action` → attach as OCI referrer.
### 0.4 Fix DistributedCache Layer Replication
In `src/cache_cluster.rs`: `DistributedCache::put_layer` must replicate to replica nodes exactly as `put` does. Current pass-through to local is a correctness bug for OCI layer sharing.
***
## Phase 1 — Secure Transport Layer (v0.5.0) — 4 weeks
**Goal:** All inter-node and client-server communication is authenticated and encrypted.
### 1.1 mTLS for Cluster Nodes
Add `rustls` + `rcgen` to `Cargo.toml`. Boot-time certificate generation or cert-manager-injected volume mount.
* `src/tls.rs`: `TlsConfig` struct — loads `cert.pem`/`key.pem`/`ca.pem` from `MEMOBUILD_TLS_*` env vars.
* Axum server: wrap with `axum_server::tls_rustls::RustlsConfig`.
* reqwest clients (cluster replication, remote cache client): `ClientBuilder::use_rustls_tls().add_root_certificate(ca)`.
* cert-manager `Certificate` CRD manifest in `deploy/k8s/certs/`.
Env vars: `MEMOBUILD_TLS_CERT`, `MEMOBUILD_TLS_KEY`, `MEMOBUILD_TLS_CA`.
### 1.2 API Authentication
Add `src/auth.rs`: Axum middleware layer.
* Bearer token validation (`Authorization: Bearer <token>`) — tokens stored as Argon2-hashed values in PostgreSQL.
* Token issuance endpoint `POST /auth/token` (admin only).
* Rate limiting: `tower_governor` crate, 1000 req/min per token, 100 req/min unauthenticated.
* Audit log: every authenticated operation logged as structured JSON with `tracing::info!`.
Env var: `MEMOBUILD_ADMIN_TOKEN` for bootstrap token.
### 1.3 Secrets Management Integration
* `src/secrets.rs`: trait `SecretProvider` with implementations for:
    * `EnvSecretProvider` (default, dev only)
    * `VaultSecretProvider`: HashiCorp Vault KV v2 via `vaultrs` crate.
    * `AwsKmsProvider`: AWS KMS via `aws-sdk-kms` for registry credential encryption at rest.
* Replace all `env::var("MEMOBUILD_TOKEN")` call sites with `SecretProvider::get("registry_token")`.
### 1.4 Container Security Hardening
In `Dockerfile.server` and all K8s manifests:
* `securityContext.runAsNonRoot: true`, `runAsUser: 65532` (distroless nonroot).
* `readOnlyRootFilesystem: true`.
* `allowPrivilegeEscalation: false`.
* `capabilities.drop: ["ALL"]`.
* Seccomp profile: `RuntimeDefault`.
***
## Phase 2 — Object Storage Backend (v0.5.1) — 3 weeks
**Goal:** Decouple blob storage from local disk. Required for stateless horizontal scaling of cache nodes.
### 2.1 `ArtifactStorage` S3/GCS Backend
Add `src/storage/` module:
* `src/storage/mod.rs`: extend existing `ArtifactStorage` trait with `stream_get(&str) -> impl Stream<Item=Bytes>`.
* `src/storage/s3.rs`: `S3Storage` using `aws-sdk-s3` — multipart upload for artifacts > 5 MB, presigned URLs for direct client downloads.
* `src/storage/gcs.rs`: `GcsStorage` using `google-cloud-storage` crate.
* `src/storage/local.rs`: existing filesystem backend, retained for single-node / dev mode.
Config via `MEMOBUILD_STORAGE_BACKEND=s3|gcs|local`, `MEMOBUILD_STORAGE_BUCKET`.
MinIO compatibility: same S3 SDK, `MEMOBUILD_STORAGE_ENDPOINT` override.
### 2.2 Redis L1 Distributed Cache
Add `src/cache/redis.rs`: `RedisCache` implementing `RemoteCache` via `fred` async Redis client.
* Hot path: cache node checks Redis before hitting object storage. Cache TTL configurable.
* Invalidation: `PUBLISH memobuild:evict:<hash>` on GC.
Config: `MEMOBUILD_REDIS_URL=redis://localhost:6379`.
### 2.3 Automated Garbage Collection
* `src/gc.rs`: `GarbageCollector` with configurable retention policy (age-based + LRU size-based).
* Tokio scheduled task: runs every 6 hours by default (`MEMOBUILD_GC_INTERVAL_HOURS`).
* GC respects replication factor: only delete artifact from object storage when confirmed absent from all nodes.
* Expose GC status via `GET /gc/status`.
***
## Phase 3 — Full Observability Stack (v0.6.0) — 3 weeks
**Goal:** Every production metric, trace, and alert is defined in code alongside the source.
### 3.1 Prometheus Metrics Endpoint
Add `prometheus` + `prometheus-client` crates.
`src/metrics.rs`: global `MetricsRegistry` with labeled counters/histograms:
* `memobuild_cache_hits_total{tier,node_id}`
* `memobuild_cache_misses_total{tier,node_id}`
* `memobuild_build_duration_seconds{status}` histogram (buckets: 0.1, 0.5, 1, 5, 30, 60, 300)
* `memobuild_cluster_nodes_total{region,status}`
* `memobuild_replication_lag_seconds` gauge
* `memobuild_artifact_size_bytes` histogram
* `memobuild_gc_deleted_total` counter
Axum route `GET /metrics` → Prometheus text format (no auth, restricted to cluster-internal NetworkPolicy).
### 3.2 OpenTelemetry Distributed Tracing
Add `opentelemetry`, `opentelemetry-otlp`, `tracing-opentelemetry` crates.
Instrumentation points:
* `span!("build.dag.execute")` wrapping entire DAG run, child spans per node.
* `span!("cache.lookup")` with `cache.tier` attribute.
* `span!("cluster.replicate")` with target node IDs.
* `span!("oci.push")` with registry URL and layer count.
Trace context propagated via `traceparent` HTTP header in all inter-node calls.
Exporter: OTLP to Jaeger / Grafana Tempo (`OTEL_EXPORTER_OTLP_ENDPOINT`).
### 3.3 Grafana Dashboards as Code
`deploy/grafana/dashboards/memobuild-cluster.json` — provisioned via `grafana/provisioning/`:
* Cache hit/miss rate by tier and node.
* Build throughput and P99 latency time series.
* Cluster node health heatmap.
* Auto-scaler replica count vs queue depth.
* Object storage I/O throughput.
`deploy/prometheus/rules/memobuild.yml` — alerting rules:
* `CacheNodeDown`: any node unhealthy > 2 min → PagerDuty/Slack.
* `ReplicationLagHigh`: lag > 30s → warning.
* `BuildQueueSaturated`: queued builds > 500 for > 5 min → critical.
* `DiskUsageHigh`: cache partition > 85% → warning.
* `ErrorRateHigh`: HTTP 5xx > 5% rate over 5 min → critical.
***
## Phase 4 — Kubernetes-Native Operator (v0.7.0) — 5 weeks
**Goal:** MemoBuild cluster lifecycle managed by a K8s operator, eliminating manual YAML.
### 4.1 Custom Resource Definitions
`deploy/k8s/crds/memobuildcluster.yaml`:
```yaml
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: memobuildclusters.build.nrelab.io
spec:
  group: build.nrelab.io
  versions:
  - name: v1alpha1
    served: true
    storage: true
  scope: Namespaced
  names:
    plural: memobuildclusters
    singular: memobuildcluster
    kind: MemoBuildCluster
```
Spec fields: `replicas`, `storageBackend` (s3/gcs/local), `tlsSecretRef`, `postgresRef`, `redisRef`, `scalingPolicy`.
### 4.2 Operator Implementation
`src/operator/` module using `kube-rs` controller-runtime:
* Reconcile loop: desired state (CRD) → actual state (StatefulSet + Service + ConfigMap + Secret).
* Manages `StatefulSet` for cluster nodes with stable DNS (`node-{n}.memobuild.{ns}.svc`).
* Patches HPA based on `scalingPolicy` in CRD.
* Emits K8s Events on scale-up/down, node failures, GC runs.
* Leader election via `kube::runtime::LeaderElection` — operator itself is HA.
### 4.3 Production K8s Manifests
`deploy/k8s/` directory:
* `statefulset.yaml`: `podManagementPolicy: Parallel`, `updateStrategy: RollingUpdate`, `maxUnavailable: 1`.
* `pdb.yaml`: `PodDisruptionBudget` — `minAvailable: 2` for ≥3 replica deployments.
* `hpa.yaml`: custom metric `memobuild_queued_builds` via Prometheus Adapter.
* `networkpolicy.yaml`: ingress only from `memobuild` namespace + designated CI runner namespace; egress only to PostgreSQL, Redis, object storage.
* `priorityclass.yaml`: `PriorityClass` `memobuild-cluster` value `1000000` — prevents eviction under node pressure.
* `podsecuritypolicy.yaml` / `podsecurityadmission` labels: `restricted` profile.
### 4.4 Helm Chart
`charts/memobuild/` with full `values.yaml`:
* `image.repository`, `image.tag`, `image.pullPolicy`.
* `cluster.replicas`, `cluster.replicationFactor`.
* `storage.backend`, `storage.s3.*`, `storage.gcs.*`.
* `tls.enabled`, `tls.certManager.enabled`, `tls.certManager.issuerRef`.
* `postgresql.enabled` (subchart: `bitnami/postgresql`).
* `redis.enabled` (subchart: `bitnami/redis`).
* `monitoring.enabled` — deploys `ServiceMonitor` for Prometheus Operator.
* `autoscaling.enabled`, `autoscaling.minReplicas`, `autoscaling.maxReplicas`.
`charts/memobuild/templates/`: StatefulSet, Services (headless + ClusterIP), ConfigMap, RBAC, NetworkPolicy, PDB, HPA, CRD install hook.
***
## Phase 5 — Supply Chain Security & SLSA Compliance (v0.8.0) — 4 weeks
**Goal:** Every build artifact is signed, attested, and auditable. SLSA Level 3 achieved.
### 5.1 SLSA Provenance Generation
`src/slsa.rs`: `ProvenanceGenerator` producing SLSA `BuildDefinition` + `RunDetails` per build.
* Captures: source URI + digest, builder ID (node ID + container image digest), build invocation parameters, environment variables (sanitized).
* Format: `in-toto` attestation bundle (JSON envelope, DSSE signature).
* Stored as OCI referrer attached to built image manifest.
CLI flag: `memobuild build --provenance` (default on in production mode).
### 5.2 Cosign Artifact Signing
Add `cosign` binary integration (or `sigstore` Rust crate when stable).
* Keyless signing flow: OIDC token from K8s ServiceAccount → Fulcio CA → Rekor transparency log entry.
* Signing on every `memobuild build --push` call.
* Verification: `src/verify.rs` — `memobuild verify <image>` command checks Rekor log + Cosign bundle.
* Policy: configurable whether unsigned images can be pulled from remote cache (`MEMOBUILD_REQUIRE_SIGNED=true`).
### 5.3 SBOM Generation
`src/sbom.rs`: generates CycloneDX 1.5 SBOM for built OCI images.
* Lists all `COPY`-ed files with their content hashes.
* References resolved package manager lockfiles (package-lock.json, Cargo.lock, go.sum).
* Attached to image as OCI referrer.
CLI: `memobuild sbom <image>` — outputs CycloneDX JSON to stdout.
### 5.4 Sigstore Policy Controller
`deploy/k8s/policy/sigstore-policy.yaml`: ClusterImagePolicy (Sigstore Policy Controller) enforcing that any image built by MemoBuild has a valid Rekor entry before admission into the cluster.
### 5.5 Audit Trail
`src/audit.rs`: immutable append-only audit log.
* Every cache read/write, node join/leave, GC run, scaling event written as structured NDJSON.
* Stored in PostgreSQL `audit_log` table with row-level SHA256 chain hash (each row hashes previous row's hash — tamper-evident).
* Export: `memobuild audit export --since <date>` → NDJSON or CSV.
***
## Phase 6 — gRPC Build Protocol & Remote Execution API (v0.9.0) — 5 weeks
**Goal:** Replace HTTP/JSON execution protocol with gRPC streaming. Achieve compatibility with Bazel RE API.
### 6.1 gRPC Execution Service
Add `tonic` (already in `Cargo.toml` as optional) to required deps. Enable `remote-exec` feature by default.
`proto/memobuild/v1/execution.proto`:
* `ExecutionService.Execute(ExecuteRequest) returns (stream ExecuteResponse)` — streaming build log lines.
* `ExecutionService.WaitExecution(WaitExecutionRequest) returns (stream ExecuteResponse)` — reconnect support.
* `CacheService.GetActionResult(GetActionResultRequest) returns (ActionResult)` — RE API compatible.
* `CacheService.UpdateActionResult(...)` — store results.
* `ContentAddressableStorageService.FindMissingBlobs / BatchReadBlobs / BatchUpdateBlobs / GetTree` — full REAPI CAS.
### 6.2 Bazel RE API Compatibility
Implement `google.devtools.remoteexecution.v2` proto service surface in `src/remote_exec/reapi.rs`.
This enables any Bazel/Buck2/Pants build using `--remote_executor` to use MemoBuild as the execution backend.
### 6.3 Build Sandboxing in Workers
`src/sandbox/linux.rs`: use `landlock` (Linux kernel LSM via `landlocked` crate) + Linux namespaces (`unshare`) for sandboxing worker build steps.
* Each task runs in new mount/pid/net/user namespace.
* Network: blocked by default unless `--allow-network` specified.
* Filesystem: worker task gets a temporary overlay mount on the workspace.
macOS fallback: `sandbox/macos.rs` using `sandbox-exec` profiles.
***
## Phase 7 — Multi-Tenancy & Enterprise (v1.0.0) — 5 weeks
**Goal:** Org-isolated cache namespaces, quotas, billing hooks, admin portal.
### 7.1 Cache Namespace Isolation
`src/tenancy.rs`:
* Every cache key prefixed with `{org_id}/{project_id}/`. Tenants cannot read/write other tenants' artifacts.
* PostgreSQL RLS (Row-Level Security) policies enforce isolation at DB layer.
* Redis keyspace: `{org_id}:{hash}` prefix with per-org TTL policies.
* Object storage: per-org S3 prefix + optional per-org KMS key for at-rest encryption.
### 7.2 Resource Quotas
* PostgreSQL table `org_quotas`: `max_cache_bytes`, `max_concurrent_builds`, `max_artifact_ttl_days`.
* Quota enforcement middleware in `src/auth.rs`.
* K8s `ResourceQuota` + `LimitRange` per tenant namespace when using the Operator.
* Quota exceeded: HTTP 429 with `Retry-After` header + audit event.
### 7.3 Admin REST API
`src/admin/mod.rs` — routes prefixed `/admin/v1/` (requires admin token):
* `POST /admin/v1/orgs` — create org.
* `GET /admin/v1/orgs/{id}/usage` — cache bytes, build count, last active.
* `POST /admin/v1/orgs/{id}/tokens` — issue org-scoped token.
* `DELETE /admin/v1/orgs/{id}` — purge org data (GDPR right-to-erasure).
* `POST /admin/v1/gc` — trigger GC for specific org.
### 7.4 Global CDN Distribution
* `src/cdn.rs`: `CdnBackend` trait — presigned URL generation pointing to CloudFront/Fastly.
* Build client redirected to presigned URL for artifact download (avoids proxying large blobs through cluster).
* Cache-Control headers set on object storage objects for CDN edge caching.
* Multi-region: cache node in each region writes to regional bucket; cross-region replication handled by S3 CRR or GCS dual-region.
### 7.5 Developer Portal (Web UI)
`extension/` directory (JavaScript/TypeScript, already present):
* Extend existing WebSocket dashboard to full SPA using Svelte or Solid.js.
* Pages: Cluster health, Build history, Cache analytics, Org management, Token management, Audit log viewer.
* Packaged as separate OCI image `nrelab/memobuild-portal:latest`.
***
## Production SLO Targets by Phase
**P0 (v0.4.1):** MemoBuild server image ≤ 15 MB, multi-arch, signed.
**P1 (v0.5.0):** Zero plaintext inter-node traffic. All endpoints require auth.
**P2 (v0.5.1):** Artifact storage fully decoupled from local disk. GC automated.
**P3 (v0.6.0):** 100% of builds traced end-to-end. Alerting latency < 1 min.
**P4 (v0.7.0):** Cluster deployed and upgraded with zero manual YAML. PDB prevents split-brain.
**P5 (v0.8.0):** SLSA Level 3 on all `--push` builds. Every artifact signed + SBOMed.
**P6 (v0.9.0):** Bazel RE API compatible. Build tasks sandboxed. Streaming log tailing.
**P7 (v1.0.0):** Multi-tenant isolation enforced at DB+storage+K8s layers. CDN-accelerated artifact delivery. 99.95% uptime SLA.
***
## Implementation Priority Matrix
**Must-do before any production traffic (P0–P2):**
* Multi-stage production Dockerfile (P0.1)
* Docker Compose full-stack for dev (P0.2)
* Fix DistributedCache layer replication bug (P0.4)
* mTLS between cluster nodes (P1.1)
* API authentication + rate limiting (P1.2)
* Object storage backend (P2.1)
* Automated garbage collection (P2.3)
**High value, schedule for v0.6–v0.7 (P3–P4):**
* Prometheus `/metrics` endpoint + Grafana dashboards (P3.1–P3.3)
* NetworkPolicy + PDB + PriorityClass manifests (P4.3)
* Helm chart (P4.4)
**Enterprise features, v0.8–v1.0 (P5–P7):**
* SLSA provenance + Cosign signing (P5.1–P5.2)
* gRPC execution service (P6.1)
* Multi-tenancy + admin API (P7.1–P7.3)
***
## Key New Dependencies to Add
* `rustls` + `axum-server` with rustls — mTLS
* `rcgen` — self-signed cert generation for dev
* `tower_governor` — rate limiting
* `prometheus-client` — metrics
* `opentelemetry` + `opentelemetry-otlp` + `tracing-opentelemetry` — tracing
* `fred` — async Redis client
* `aws-sdk-s3` + `google-cloud-storage` — object storage
* `vaultrs` — Vault secret provider
* `tonic` (enable existing optional dep) — gRPC
* `landlocked` — Linux sandboxing for workers
* `in-toto` / `sigstore` — SLSA attestation
***
## Deliverables Checklist per Phase
**P0:** `Dockerfile.server`, `Dockerfile.worker`, `docker-compose.prod.yml`, `.env.example`, `.github/workflows/docker.yml`, `scripts/db/init.sql`.
**P1:** `src/tls.rs`, `src/auth.rs`, `src/secrets.rs`, `deploy/k8s/certs/`, updated all HTTP clients.
**P2:** `src/storage/`, `src/cache/redis.rs`, `src/gc.rs`, MinIO in compose.
**P3:** `src/metrics.rs`, OpenTelemetry instrumentation in core/cache/cluster, `deploy/grafana/`, `deploy/prometheus/rules/`.
**P4:** `src/operator/`, `deploy/k8s/crds/`, full `deploy/k8s/` manifest set, `charts/memobuild/`.
**P5:** `src/slsa.rs`, `src/sbom.rs`, `src/verify.rs`, `src/audit.rs`, `deploy/k8s/policy/`.
**P6:** `proto/memobuild/v1/`, gRPC server, RE API compatibility layer, `src/sandbox/`.
**P7:** `src/tenancy.rs`, `src/admin/`, `src/cdn.rs`, extended portal SPA.