#!/bin/bash
set -e

echo "🏗️  Starting MemoBuild Farm demo..."

# Build binary
cargo build --features remote-exec

# Create a demo Dockerfile that uses Mac-friendly commands
echo "RUN echo 'Hello from Build Farm'" > Dockerfile.farm
echo "RUN ls -R" >> Dockerfile.farm

# 1. Start Worker in background
echo "👷 Starting Worker Node on port 9001..."
MEMOBUILD_WORKER_ID=worker-alpha ./target/debug/memobuild --worker --port 9001 > worker.log 2>&1 &
WORKER_PID=$!

# 2. Wait for worker to start
sleep 3

# 3. Start Scheduler in background
echo "🚀 Starting Scheduler on port 9000..."
MEMOBUILD_WORKERS=http://localhost:9001 MEMOBUILD_STRATEGY=DataLocality ./target/debug/memobuild --scheduler --port 9000 > scheduler.log 2>&1 &
SCHEDULER_PID=$!

# 4. Wait for scheduler to start
sleep 3

# 5. Run a build using the remote farm
echo "📡 Running MemoBuild command using the build farm..."
# We use a custom Dockerfile to avoid 'apk' errors on local Mac workers
MEMOBUILD_REMOTE_EXEC=http://localhost:9000 ./target/debug/memobuild --reproducible > client.log 2>&1 || true

# 6. Cleanup
echo "🧹 Cleaning up..."
kill $WORKER_PID $SCHEDULER_PID || true

echo "✨ Demo complete!"
echo "--- Client Log ---"
grep "RemoteExec" client.log
echo "--- Scheduler Log ---"
grep "execute" scheduler.log
echo "--- Worker Log ---"
grep "Received" worker.log
