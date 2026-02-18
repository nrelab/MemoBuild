# 📄 MemoBuild Technical Whitepaper

## Abstract
MemoBuild is a high-performance, strictly incremental container build engine designed to overcome the limitations of linear layer-based build systems. By representing build processes as Directed Acyclic Graphs (DAGs) and utilizing content-addressed storage (CAS), MemoBuild ensures that only the minimal necessary work is performed for any given change. This document outlines the mathematical foundations, architectural components, and optimization strategies employed by the engine.

---

## 1. Mathematical Foundation: Content-Addressed DAGs

### 1.1 The Docker Limitation
Traditional container builds follow a sequential model: $L_n = f(L_{n-1}, I_n)$, where $L$ is a layer and $I$ is an instruction. Any change in $I_k$ invalidates all layers $L_j$ for $j \ge k$.

### 1.2 The MemoBuild Model
MemoBuild models the build as a set of nodes $N$ in a DAG. Each node $n \in N$ has a unique identity defined by its **Input Hash** ($H_{in}$):

$$H_{node} = \text{BLAKE3}(H_{inputs} \parallel H_{instruction} \parallel H_{context})$$

Where:
- $H_{inputs}$: The hashes of all parent nodes (dependencies).
- $H_{instruction}$: The hash of the command string (e.g., `RUN npm install`).
- $H_{context}$: The fingerprint of any files from the host filesystem utilized by the node (e.g., `COPY . .`).

### 1.3 Determinism and Reproducibility
MemoBuild enforces determinism by capturing the execution environment (environment variables, user IDs, timestamps) into the node hash. If the environment changes, the hash changes, triggering a rebuild.

---

## 2. Filesystem Fingerprinting Engine

### 2.1 Fast Path Selection
MemoBuild utilizes a highly optimized "Walker" that uses parallel directory traversal (Rayon) to generate fingerprints.

### 2.2 BLAKE3 Hashing
We selected BLAKE3 for its extreme performance on modern multi-core CPUs, allowing MemoBuild to hash large workspaces in milliseconds, which is the primary bottleneck in large-scale builds.

### 2.3 Ignore Semantics
Full support for `.dockerignore` and `.gitignore` patterns is baked into the hashing core, ensuring that temporary or sensitive files never affect the build cache.

---

## 3. Tiered Caching Strategy

MemoBuild implements a three-tier cache to maximize reuse across different environments:

| Tier | Type | Scope | Latency |
| :--- | :--- | :--- | :--- |
| **L1** | In-Memory | Process | Microseconds |
| **L2** | Local Disk | User/Machine | Milliseconds |
| **L3** | Remote HTTP | Team/Global | Network Bound |

### 3.1 Content-Addressed Storage (CAS)
Each build artifact is stored in a CAS where the key is the BLAKE3 hash of the node. Artifacts are compressed using Gzip for efficient storage and transfer.

---

## 4. OCI Image Construction

Instead of modifying existing container layers, MemoBuild constructs images by:
1. Generating individual tarballs for each "dirty" node output.
2. Calculating the diff-id and digest for each layer.
3. Building an OCI-compliant manifest and configuration JSON.
4. Pushing blobs directly to the registry using the OCI Distribution Spec.

---

## 5. Distributed Build Orchestration

MemoBuild's server acts as a centralized metadata and artifact store. 

### 5.1 Analytics and Webhooks
The server tracks build performance (duration, cache hit rate) and can notify external consumers (Slack, CI dashboards) via webhooks upon build completion.

### 5.2 Kubernetes Integration
MemoBuild can generate native Kubernetes Job manifests, allowing builds to be offloaded to elastic cloud infrastructure while still benefiting from the shared remote cache.

---

## 6. Conclusion
MemoBuild represents a paradigm shift in container builds, moving away from fragile linear layers toward a robust, mathematical dependency graph. The result is a system that is not only faster but fundamentally more reliable and easier to scale.
