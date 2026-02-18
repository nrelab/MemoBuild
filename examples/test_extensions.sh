#!/bin/bash
set -e

echo "🏗️  Testing Docker Extensions..."
rm -rf .memobuild-cache ~/.memobuild/cache

# Create Dockerfile with extensions
cat <<EOF > Dockerfile.ext
FROM scratch
WORKDIR /app

# Standard instruction (using sh compatible echo)
RUN echo "Standard RUN"

# Extended instructions
RUN_EXTEND echo "Parallel Extended RUN 1"
RUN_EXTEND echo "Parallel Extended RUN 2"

# Extended COPY
# Create dummy files
RUN echo "file1" > file1.txt
RUN mkdir -p data
RUN echo "file2" > data/file2.txt

COPY_EXTEND file1.txt dest1.txt tag1 tag2
COPY_EXTEND data dest_data tag3

# Hook
HOOK echo "Build Started"
EOF

# Build using memobuild
echo "🚀 Running memobuild..."
cargo run -- --file Dockerfile.ext --reproducible > extension_build.log 2>&1

echo "✅ Build completed. Checking logs for extensions..."

if grep -q "Executing extended RUN: echo \"Parallel Extended RUN 1\"" extension_build.log; then
    echo "✅ RUN_EXTEND detected"
else
    echo "❌ RUN_EXTEND missing"
    exit 1
fi

if grep -q "Executing extended COPY" extension_build.log; then
    echo "✅ COPY_EXTEND detected"
else
    echo "❌ COPY_EXTEND missing"
    exit 1
fi

if grep -q "Running custom hook: echo" extension_build.log; then
    echo "✅ HOOK detected"
else
    echo "❌ HOOK missing"
    exit 1
fi

echo "✨ Docker Extensions test passed!"
rm Dockerfile.ext file1.txt
rm -rf data
