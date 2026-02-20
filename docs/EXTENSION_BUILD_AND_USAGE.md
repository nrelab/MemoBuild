# MemoBuild Docker Extension - Build and Usage Guide

## Overview

The MemoBuild Docker Extension provides an incremental build farm system that integrates directly into Docker Desktop. This guide covers how to build, install, and use the extension.

## Prerequisites

- Docker Desktop with Extensions support (version 4.15+)
- Go 1.24+ (for local development)
- Node.js 24+ (for UI development)
- Make (for build automation)

## Project Structure

```
extension/
├── backend/           # Go backend service
│   ├── go.mod
│   ├── go.sum
│   └── main.go
├── ui/               # React frontend
│   ├── package.json
│   ├── src/
│   └── public/
├── Dockerfile        # Multi-stage build for extension
├── Makefile         # Build automation
├── metadata.json    # Extension metadata
├── docker-compose.yaml
└── README.md
```

## Building the Extension

### Local Development Build

#### Using Make (Recommended)

```bash
# Build the extension image
make build-extension

# Install the extension locally
make install-extension

# Update an existing installation
make update-extension
```

#### Manual Docker Build

```bash
# Build from project root
docker build --tag=khulnasoft/memobuild-extension:latest -f extension/Dockerfile .

# Install the extension
docker extension install khulnasoft/memobuild-extension:latest

# Update the extension
docker extension update khulnasoft/memobuild-extension:latest
```

### Multi-Platform Build

For production releases with multi-architecture support:

```bash
# Create buildx builder (if not exists)
make prepare-buildx

# Build and push to registry
make push-extension tag=v1.0.0
```

## CI/CD Pipeline

The extension uses GitHub Actions for automated building and releasing:

### Workflow Triggers

- **Main Branch Push**: Builds and tests on every push to main
- **Pull Requests**: Validates changes before merging
- **Release Events**: Automatically builds and publishes releases
- **Manual Dispatch**: On-demand builds

### Build Process

1. **Multi-Platform Build**: Builds for linux/amd64 and linux/arm64
2. **Container Registry**: Pushes to GitHub Container Registry (ghcr.io)
3. **Semantic Versioning**: Automatic tagging based on git tags
4. **Release Notes**: Generates installation instructions

### Installation from CI/CD

```bash
# Install latest version
docker extension install ghcr.io/nrelab/memobuild-extension:latest

# Install specific version
docker extension install ghcr.io/nrelab/memobuild-extension:v1.0.0

# Update to latest
docker extension update ghcr.io/nrelab/memobuild-extension:latest
```

## Usage Guide

### Accessing the Extension

1. **Docker Desktop Integration**: The extension appears as a new tab in Docker Desktop
2. **Web UI**: Access the dashboard through the Docker Desktop interface
3. **CLI Integration**: Use the `memobuild` command (stub included in extension)

### Features

- **Incremental Build Farm**: Manages distributed build processes
- **Real-time Monitoring**: Track build progress and metrics
- **Cache Management**: Optimizes build times with intelligent caching
- **DAG Visualization**: View build dependency graphs
- **Resource Monitoring**: Track CPU, memory, and disk usage

### Configuration

The extension can be configured through:

1. **Docker Desktop Settings**: Extension-specific configuration
2. **Environment Variables**: Runtime configuration
3. **Configuration Files**: Persistent settings

### API Integration

The extension exposes a Unix socket for API communication:

```bash
# Socket location
/run/guest-services/backend.sock
```

## Development Workflow

### Backend Development (Go)

```bash
cd extension/backend

# Install dependencies
go mod download

# Run locally
go run main.go

# Build binary
go build -o service main.go
```

### Frontend Development (React)

```bash
cd extension/ui

# Install dependencies
npm ci --legacy-peer-deps

# Start development server
npm start

# Build for production
npm run build
```

### Testing

```bash
# Run backend tests
cd extension/backend
go test ./...

# Run frontend tests
cd extension/ui
npm test

# Integration tests
docker-compose -f extension/docker-compose.yaml up --abort-on-container-exit
```

## Troubleshooting

### Common Issues

1. **Extension Installation Fails**
   - Ensure Docker Desktop is updated to the latest version
   - Check that Extensions are enabled in Docker Desktop settings
   - Verify the extension image is accessible

2. **Build Failures**
   - Check Go and Node.js versions meet requirements
   - Ensure all dependencies are installed
   - Verify Docker daemon is running

3. **Runtime Errors**
   - Check Docker Desktop logs for extension errors
   - Verify socket permissions
   - Ensure sufficient system resources

### Debug Commands

```bash
# Check extension status
docker extension ls

# View extension logs
docker extension logs memobuild-extension

# Inspect extension container
docker inspect $(docker ps -q --filter "name=memobuild")

# Remove extension
docker extension rm memobuild-extension
```

## Release Process

### Creating a New Release

1. **Update Version**: Update version numbers in relevant files
2. **Tag Release**: Create and push a git tag

```bash
git tag v1.0.0
git push origin v1.0.0
```

3. **GitHub Release**: Create a release on GitHub (triggers CI/CD)
4. **Verify Build**: Check that the extension builds successfully
5. **Test Installation**: Install the released version

### Version Management

- **Semantic Versioning**: Follow MAJOR.MINOR.PATCH format
- **Release Tags**: Use `v*` pattern for automatic CI/CD triggers
- **Backward Compatibility**: Maintain API compatibility within major versions

## Contributing

### Development Setup

1. Clone the repository
2. Install Docker Desktop with Extensions support
3. Set up Go and Node.js development environments
4. Follow the local development build instructions

### Submitting Changes

1. Create a feature branch
2. Make changes and test locally
3. Submit a pull request
4. CI/CD will automatically test and validate changes

## Support

- **Documentation**: Check this guide and inline documentation
- **Issues**: Report bugs via GitHub Issues
- **Community**: Join discussions in GitHub Discussions
- **Logs**: Check Docker Desktop extension logs for troubleshooting

## Security Considerations

- **Container Security**: Extension runs in isolated containers
- **Network Access**: Limited to required services only
- **File System**: Restricted access to Docker Desktop sandbox
- **API Security**: Socket-based communication with proper permissions

## Performance Optimization

- **Build Caching**: Leverage Docker layer caching
- **Resource Limits**: Configure appropriate resource constraints
- **Monitoring**: Use built-in metrics for performance tuning
- **Updates**: Regularly update dependencies for security and performance
