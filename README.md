# 🧠 MemoBuild Engine

**High-Performance Incremental Build System with Smart Caching**

MemoBuild is a next-generation build system that intelligently rebuilds only what's changed, using advanced dependency tracking, multi-layer caching, and OCI-compatible image generation.

## 🏗️ Architecture Overview

```
┌───────────────────────────┐
│        CLI / API          │
│   (memobuild build/run)   │
└────────────┬──────────────┘
             │
┌────────────▼──────────────┐
│     Build Orchestrator     │
└───────┬─────────┬─────────┘
        │         │
 ┌──────▼───┐   ┌─▼─────────────────┐
 │ Change   │   │ Graph Builder     │
 │ Detector │   │ (Dockerfile→DAG)  │
 └──────┬───┘   └──────┬────────────┘
        │              │
        └────────┬─────┘
                 ▼
    ┌──────────────────────────┐
    │   Smart Rebuild Engine   │
    │ (dirty + propagation)    │
    └──────────┬───────────────┘
               │
     ┌─────────▼─────────┐      ┌────────────────────────┐
     │ Execution Engine  │ <──> │  Remote Cache Server   │
     │ (parallel DAG)    │      │  (Axum + SQLite + FS)  │
     └───────┬───────────┘      └────────────────────────┘
             │
 ┌───────────▼───────────────────┐
 │   Hybrid Cache System         │
 │  (local disk + remote HTTP)   │
 └───────────┬───────────────────┘
             │
     ┌───────▼────────┐
     │ OCI Image Gen  │
     └───────┬────────┘
             │
┌────────────▼────────────┐
│ containerd / registry   │
│ (OCI compatible output) │
└─────────────────────────┘
```

## ✨ Features

### 🎯 Smart Incremental Builds
- **BLAKE3 Hashing**: Ultra-fast content hashing for change detection
- **Dependency Tracking**: Automatic propagation of changes through the build graph
- **Minimal Rebuilds**: Only rebuild what's actually changed

### 🚀 Performance
- **Parallel Execution**: Execute independent build steps concurrently using Rayon
- **Hybrid Cache**: Tiered caching (Local L1 + Remote L2) for speed and sharing
- **Topological Ordering**: Optimal execution order based on dependency graph
- **Remote Cache Server**: Shared distributed cache for teams and CI/CD

### 📦 OCI Compatibility
- **Standard Output**: Generate OCI-compliant images
- **Docker Compatible**: Works with Docker, containerd, and Kubernetes
- **Layer Management**: Efficient layer creation and digest calculation

### 🔄 Build State Machine

```
INIT
 → SCAN_FILES
 → HASH_COMPUTE
 → GRAPH_BUILD
 → DIRTY_MARK
 → PROPAGATE
 → EXECUTE
 → CACHE_STORE
 → EXPORT_IMAGE
 → DONE
```

## 🚀 Quick Start

### Installation

```bash
# Clone the repository
cd memobuild

# Build the project (requires Rust)
cargo build --release

# Run MemoBuild
cargo run -- build
```

### Remote Cache (Optional)

You can share build artifacts across your team or CI/CD by running a remote cache server.

```bash
# Start the Remote Cache Server
cargo run --features server -- --server --port 8080

# Build using the Remote Cache
MEMOBUILD_REMOTE_URL=http://localhost:8080 memobuild build
```

### Basic Usage

```bash
# Build from default Dockerfile
memobuild build

# Build from custom Dockerfile
memobuild build custom.Dockerfile

# Show cache information
memobuild info

# Clean cache
memobuild clean
```

## 📋 Core Components

### 1. **Change Detector** (`src/core.rs`)
- BLAKE3-based file hashing
- Directory tree hashing
- Dependency-aware hash computation
- Dirty flag propagation

### 2. **Graph Builder** (`src/graph.rs`)
- Dockerfile → DAG conversion
- Node types: Source, Build, Artifact, Dependency
- Topological sorting
- Dependency management

### 3. **Hybrid Cache System** (`src/cache.rs`)
- Tiered lookup: Local Disk -> Remote HTTP -> Build
- Automatic artifact upload to remote on successful build
- `LocalCache`: Local persistent storage
- `HttpRemoteCache`: Remote storage integration via `reqwest`

### 4. **Remote Cache Server** (`src/server/`)
- **Axum Web Server**: High-performance HTTP controller
- **SQLite Metadata**: Fast entry tracking with hit/miss analytics
- **Sharded Storage**: Content-addressed filesystem layout (ab/cd/...)

### 4. **Executor** (`src/executor.rs`)
- Sequential execution
- Parallel execution (Rayon)
- Level-based parallelism
- Cache integration

### 5. **Dockerfile Parser** (`src/dockerfile.rs`)
- Supports: FROM, COPY, RUN, WORKDIR, ENV, CMD, EXPOSE
- Instruction validation
- Error handling

### 6. **OCI Exporter** (`src/oci.rs`)
- OCI manifest generation
- Layer tarball creation
- SHA256 digest calculation
- Config JSON generation

## 🔌 Protocol Specifications

### Node Definition
```json
{
  "id": "node-uuid",
  "type": "source|dependency|build|artifact",
  "inputs": ["nodeA", "nodeB"],
  "command": "npm install",
  "env": {},
  "hash": "blake3-hash",
  "dirty": false
}
```

### Cache Object
```json
{
  "cache_key": "hash(node)",
  "created_at": "timestamp",
  "artifact_path": "/cache/objects/abc123",
  "size": 123456,
  "layer_digest": "sha256:...."
}
```

### OCI Manifest
```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.manifest.v1+json",
  "config": {
    "mediaType": "application/vnd.oci.image.config.v1+json",
    "digest": "sha256:...",
    "size": 1234
  },
  "layers": [...]
}
```

## ⚙️ Core Algorithm

```rust
// 1. Scan and hash files
scan_files()
compute_hashes()

// 2. Build dependency graph
build_dependency_graph()

// 3. Mark dirty nodes
for node in graph:
    if hash_changed:
        mark_dirty(node)

// 4. Propagate dirty flags
propagate_dirty()

// 5. Execute with caching
for node in topological_order:
    if node.dirty:
        if cache_hit:
            load_from_cache()
        else:
            execute()
            store_in_cache()
    else:
        load_from_cache()
```

## 📊 Example Build Flow

```
📄 Parsing Dockerfile: Dockerfile.sample
📊 Build graph created with 9 nodes
🔍 Detecting changes...
🔄 Propagating dirty flags...
🎯 3 nodes need rebuilding
⚡ Executing build...
  ⚡ [0] FROM node:18-alpine (cached)
  ⚡ [1] WORKDIR /app (cached)
  ⚡ [2] COPY package.json /app/ (cached)
  🔧 [3] RUN npm install (rebuilding)
  🔧 [4] COPY src /app/src (rebuilding)
  🔧 [5] RUN npm run build (rebuilding)
  ✓ [6] ENV NODE_ENV=production (unchanged)
  ✓ [7] EXPOSE 3000 (unchanged)
  ✓ [8] CMD node dist/index.js (unchanged)
📦 Exporting OCI image...
  📁 Creating image directory: .memobuild-output/memobuild-output-latest
  ✅ Config created: sha256:abc123...
  ✅ Manifest created
  📊 Total layers: 9
✅ Build completed successfully
🎉 Image ready: memobuild-output:latest
```

## 🎯 Next Evolution Steps

### Phase 1: Core Enhancements ✅
- [x] Dockerfile parser → DAG builder
- [x] Real filesystem hashing (BLAKE3)
- [x] Parallel execution (Rayon)
- [x] OCI image exporter
- [x] Local cache system

### Phase 2: Advanced Features ✅
- [x] Remote cache server (HTTP API)
- [x] Distributed build caching
- [x] Hybrid Cache (Local + Remote)
- [x] Build artifact compression (Gzip)
- [x] Layer deduplication (Content-addressed)
- [x] Incremental layer updates (Optimized uploads)

### Phase 3: Optimization ✅
- [x] Content-addressable storage (Integrity verification)
- [x] Build cache garbage collection (GC)
- [x] Parallel layer uploads & execution (Rayon)
- [ ] Smart prefetching
- [ ] Build analytics

### Phase 4: Integration 📋
- [ ] Docker registry push/pull
- [ ] Kubernetes integration
- [ ] CI/CD pipeline support
- [ ] Build notifications
- [ ] Web dashboard

## 🧪 Testing

```bash
# Run all tests
cargo test

# Run with verbose output
cargo test -- --nocapture

# Run specific test
cargo test test_hash_str
```

## 📦 Dependencies

- **axum**: High-performance web server for remote cache
- **rusqlite**: SQLite integration for cache metadata
- **reqwest**: HTTP client for remote cache communication
- **blake3**: Ultra-fast cryptographic hashing
- **petgraph**: Graph data structures and algorithms
- **rayon**: Data parallelism
- **serde/serde_json**: Serialization
- **tar/flate2**: Archive creation
- **sha2**: SHA256 for OCI digests
- **chrono**: Timestamp handling
- **anyhow**: Error handling

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## 📄 License

MIT License - see LICENSE file for details

## 🙏 Acknowledgments

Built with inspiration from:
- Docker BuildKit
- Bazel
- Nix
- Earthly

---

**MemoBuild** - Smart builds, faster deployments 🚀
