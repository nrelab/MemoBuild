#!/bin/bash
set -e

# Setup workspace
mkdir -p test-remote-inputs
cd test-remote-inputs
echo "Hello from input file" > file1.txt

cat > Dockerfile <<EOF
FROM alpine:3.18
COPY file1.txt .
RUN cat file1.txt > dest1.txt && echo "Success"
EOF

# Kill previous processes
pkill -f "memobuild --worker" || true
pkill -f "memobuild --scheduler" || true
sleep 1

# Start Worker
cargo run --features remote-exec -- --worker --port 9101 > worker.log 2>&1 &
WORKER_PID=$!

# Start Scheduler
sleep 2
MEMOBUILD_WORKERS=http://localhost:9101 cargo run --features remote-exec -- --scheduler --port 9102 > scheduler.log 2>&1 &
SCHEDULER_PID=$!

# Run Client
sleep 2
echo "🚀 Running MemoBuild client..."
MEMOBUILD_REMOTE_EXEC=http://localhost:9102 cargo run --features remote-exec -- -f Dockerfile > client.log 2>&1 || true

# Check results
echo "--- Client Log ---"
cat client.log
echo "--- Worker Log ---"
cat worker.log

# Cleanup
kill $WORKER_PID $SCHEDULER_PID || true
cd ..
rm -rf test-remote-inputs
