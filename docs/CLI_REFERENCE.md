# 💻 MemoBuild CLI Reference

Comprehensive documentation for the `memobuild` command-line interface.

## 🛠 Basic Commands

### `memobuild build`
Main command to build an OCI image from a build context.

**Usage:**
```bash
memobuild build [PATH] [OPTIONS]
```

**Options:**
- `PATH`: Directory containing the `Dockerfile` and build context (defaults to `.`).
- `--push`: Automatically push the built image to the configured registry after success.
- `--tag <TAG>`: Specify the image tag (defaults to `latest`).
- `--remote <URL>`: Override the `MEMOBUILD_REMOTE_URL` for this build.

---

### `memobuild server`
Start the remote cache and metadata server.

**Usage:**
```bash
memobuild server [OPTIONS]
```

**Options:**
- `--port <PORT>`: Port to listen on (defaults to `8080`).
- `--storage <DIR>`: Directory for storing cache artifacts and metadata.
- `--webhook <URL>`: Optional URL to send build notifications.

---

### `memobuild push`
Manually push a locally cached artifact (by hash) to the remote registry.

**Usage:**
```bash
memobuild push <HASH>
```

---

### `memobuild pull`
Pull a base image or specific artifact layer from a remote registry.

**Usage:**
```bash
memobuild pull <IMAGE_URL>:<TAG>
```

---

### `memobuild generate-k8s`
Generates a Kubernetes Job manifest for running the current build in a cluster.

**Usage:**
```bash
memobuild generate-k8s [OPTIONS]
```

**Options:**
- `--name <JOB_NAME>`: Custom name for the K8s job.

---

### `memobuild generate-ci`
Generates a GitHub Actions workflow YAML for MemoBuild.

**Usage:**
```bash
memobuild generate-ci
```

---

## 🌐 Environment Variables

| Variable | Description | Default |
| :--- | :--- | :--- |
| `MEMOBUILD_REMOTE_URL` | URL of the remote cache server. | `None` |
| `MEMOBUILD_CACHE_DIR` | Local directory for L2 cache. | `.memobuild-cache` |
| `MEMOBUILD_REGISTRY` | Target OCI registry (e.g., `ghcr.io`). | `index.docker.io` |
| `MEMOBUILD_REPO` | Repository path (e.g., `user/app`). | `None` |
| `MEMOBUILD_TOKEN` | Authentication token for the registry. | `None` |
| `MEMOBUILD_WEBHOOK_URL` | Webhook for build notifications. | `None` |
