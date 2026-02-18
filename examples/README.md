# MemoBuild Examples

This directory contains example projects that highlight the power of MemoBuild's distributed caching and OCI image building.

## Projects

- `nodejs-app`: A simple Express.js application.
- `rust-app`: A basic async Rust application using Tokio.

## How to use with MemoBuild

### 1. Start the Remote Cache Server
In one terminal, start the MemoBuild server:
```bash
memobuild server --port 8080 --storage ./server-data
```

### 2. Configure the Client
Set the remote URL for the client:
```bash
export MEMOBUILD_REMOTE_URL=http://localhost:8080
```

### 3. Build an Example
Navigate to an example directory and run the build:
```bash
cd examples/nodejs-app
memobuild build .
```
The first build will be slow as it populates the cache. Subsequent builds (even if you clear your local cache) will be near-instant!

### 4. Build and Push OCI Image
You can also build and push directly to a registry (using the image builder logic):
```bash
export MEMOBUILD_REGISTRY=ghcr.io
export MEMOBUILD_REPO=myuser/my-app
export MEMOBUILD_TOKEN=$GITHUB_TOKEN

memobuild build --push .
```

### 5. Generate Kubernetes Job
To run this build in your cluster:
```bash
memobuild generate-k8s --name my-build-job > job.yaml
kubectl apply -f job.yaml
```

## Dashboard
Visit `http://localhost:8080` in your browser to see real-time analytics for your builds!
