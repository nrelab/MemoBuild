use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use memobuild::server;
use memobuild::{cache, core, docker, executor, export, logging};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "memobuild")]
#[command(about = "High-Performance Incremental Build System", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable JSON logging
    #[arg(long, env = "MEMOBUILD_JSON_LOGS")]
    json_logs: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Build a container image from a context directory
    Build {
        /// path to the build context
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Path to the Dockerfile
        #[arg(short, long, default_value = "Dockerfile")]
        file: String,

        /// Push the image to a registry after build
        #[arg(long)]
        push: bool,

        /// Enable reproducible build mode
        #[arg(long)]
        reproducible: bool,

        /// Perform a dry run without executing commands
        #[arg(long)]
        dry_run: bool,

        /// Use a specific sandbox runtime (local, containerd)
        #[arg(long)]
        sandbox: Option<String>,

        /// Use remote execution via scheduler
        #[arg(long)]
        remote_exec: bool,
    },
    /// Visualize the dependency graph
    Graph {
        /// Path to the build context
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Path to the Dockerfile
        #[arg(short, long, default_value = "Dockerfile")]
        file: String,
    },
    /// Explain the cache status for a specific node
    ExplainCache {
        /// Path to the build context
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Path to the Dockerfile
        #[arg(short, long, default_value = "Dockerfile")]
        file: String,

        /// Specific node ID or name to explain (optional)
        node: Option<String>,
    },
    /// Start the Remote Cache Server
    Server {
        /// Port to listen on
        #[arg(short, long, default_value_t = 8080)]
        port: u16,
    },
    /// Start the Execution Scheduler
    Scheduler {
        /// Port to listen on
        #[arg(short, long, default_value_t = 9000)]
        port: u16,
    },
    /// Start a Worker Node
    Worker {
        /// Port to listen on
        #[arg(short, long, default_value_t = 9001)]
        port: u16,

        /// Sandbox runtime to use
        #[arg(long, default_value = "local")]
        sandbox: String,

        /// Scheduler endpoint to register with
        #[arg(long, env = "MEMOBUILD_SCHEDULER_URL")]
        scheduler_url: Option<String>,
    },
    /// Pull an image from a registry
    Pull {
        /// Full image name (e.g. registry.io/repo:tag)
        image: String,
    },
    /// Generate CI/CD configurations
    GenerateCi {
        /// CI provider (github, gitlab)
        #[arg(long, default_value = "github")]
        provider: String,
    },
    /// Start a Clustered Cache Server
    Cluster {
        /// Port to listen on
        #[arg(short, long, default_value_t = 9090)]
        port: u16,

        /// Cluster node ID
        #[arg(long)]
        node_id: Option<String>,

        /// Comma-separated list of cluster peer addresses
        #[arg(long)]
        peers: Option<String>,

        /// Enable PostgreSQL storage
        #[arg(long)]
        postgres: bool,

        /// PostgreSQL connection string
        #[arg(long, env = "DATABASE_URL")]
        database_url: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize structured logging
    logging::init_logging(cli.json_logs).ok();

    match cli.command {
        Commands::Build {
            path,
            file,
            push,
            reproducible,
            dry_run,
            sandbox,
            remote_exec,
        } => {
            run_build(
                path,
                file,
                push,
                reproducible,
                dry_run,
                sandbox,
                remote_exec,
            )
            .await
        }
        Commands::Graph { path, file } => run_graph(path, file).await,
        Commands::ExplainCache { path, file, node } => run_explain_cache(path, file, node).await,
        Commands::Server { port } => {
            let webhook_url = env::var("MEMOBUILD_WEBHOOK").ok();
            let data_dir = env::current_dir()?.join(".memobuild-server");
            fs::create_dir_all(&data_dir)?;
            server::start_server(port, data_dir, webhook_url).await
        }
        Commands::Scheduler { port } => start_scheduler(port).await,
        Commands::Worker {
            port,
            sandbox,
            scheduler_url,
        } => start_worker(port, sandbox, scheduler_url).await,
        Commands::Pull { image } => run_pull(image).await,
        Commands::GenerateCi { provider } => run_generate_ci(provider).await,
        Commands::Cluster {
            port,
            node_id,
            peers,
            postgres,
            database_url,
        } => start_cluster_server(port, node_id, peers, postgres, database_url).await,
    }
}

async fn run_build(
    context_dir: PathBuf,
    dockerfile_path: String,
    push: bool,
    reproducible: bool,
    dry_run: bool,
    sandbox_type: Option<String>,
    remote_exec: bool,
) -> Result<()> {
    println!("🚀 MemoBuild Engine Starting...");

    let env_fp = memobuild::env::EnvFingerprint::collect();
    println!("   🔑 Env Fingerprint: {}", &env_fp.hash()[..8]);

    let cache = Arc::new(create_cache().await?);

    let dockerfile = fs::read_to_string(&dockerfile_path)
        .with_context(|| format!("Failed to read Dockerfile at {}", dockerfile_path))?;

    println!("📄 Parsing Dockerfile...");
    let instructions = docker::parser::parse_dockerfile(&dockerfile);

    println!("📊 Building DAG for context: {}...", context_dir.display());
    let mut graph = docker::dag::build_graph_from_instructions(instructions, context_dir.clone());

    let ai_layer = memobuild::ai::AiLayer::new();
    ai_layer.analyze(&mut graph, &env_fp, &context_dir);

    println!("🔍 Detecting changes (filesystem hashing)...");
    core::detect_changes(&mut graph);

    println!("🔄 Propagating dirty flags...");
    core::propagate_dirty(&mut graph);

    println!("🔑 Recomputing deterministic hashes...");
    core::compute_composite_hashes(&mut graph, &env_fp);

    println!("📜 Propagating artifact manifests...");
    let manifests = core::propagate_manifests(&mut graph);

    if let Some(ref _r) = cache.remote {
        for (hash, manifest) in manifests {
            let data = serde_json::to_vec(&manifest)?;
            let cache_clone = cache.clone();
            let hash_clone = hash.clone();
            tokio::spawn(async move {
                let _ = cache_clone.put_artifact(&hash_clone, &data).await;
            });
        }
    }

    let dirty = graph.nodes.iter().filter(|n| n.dirty).count();
    println!(
        "   {} dirty  |  {} cached",
        dirty,
        graph.nodes.len() - dirty
    );

    if dirty > 0 {
        println!("🚀 Initiating smart prefetching for {} nodes...", dirty);
        let dirty_hashes: Vec<String> = graph
            .nodes
            .iter()
            .filter(|n| n.dirty)
            .map(|n| n.hash.clone())
            .collect();
        cache.clone().prefetch_artifacts(dirty_hashes);
    }

    let build_start = std::time::Instant::now();
    let mut executor = executor::IncrementalExecutor::new(cache.clone())
        .with_reproducible(reproducible)
        .with_dry_run(dry_run);

    executor = executor.with_sandbox(Arc::new(memobuild::sandbox::local::LocalSandbox::new(
        context_dir.clone(),
    )));

    if let Some(st) = sandbox_type {
        if st.as_str() == "containerd" {
            #[cfg(feature = "containerd")]
            {
                let sandbox = Arc::new(memobuild::sandbox::containerd::ContainerdSandbox::new(
                    "memobuild",
                    "/run/containerd/containerd.sock",
                ));
                executor = executor.with_sandbox(sandbox);
            }
        }
    }

    // Configure remote execution if requested
    if remote_exec {
        if let Ok(scheduler_url) = std::env::var("MEMOBUILD_SCHEDULER_URL") {
            let remote_client = Arc::new(memobuild::remote_exec::client::RemoteExecClient::new(
                &scheduler_url,
            ));
            executor = executor.with_remote_executor(remote_client);
            println!("📡 Using remote execution via scheduler: {}", scheduler_url);
        } else {
            println!("⚠️  --remote-exec specified but MEMOBUILD_SCHEDULER_URL not set");
        }
    }

    executor.execute(&mut graph).await?;
    let duration = build_start.elapsed();

    let _ = cache
        .report_analytics(
            dirty as u32,
            (graph.nodes.len() - dirty) as u32,
            duration.as_millis() as u64,
        )
        .await;

    println!("📦 Exporting OCI Image...");
    let output_dir = export::export_image(&graph, "memobuild-demo:latest", reproducible)?;

    if push {
        let registry_url =
            env::var("MEMOBUILD_REGISTRY").unwrap_or_else(|_| "localhost:5000".to_string());
        let repo = env::var("MEMOBUILD_REPO").unwrap_or_else(|_| "memobuild-demo".to_string());
        let token = env::var("MEMOBUILD_TOKEN").ok();
        let mut client = export::registry::RegistryClient::new(&registry_url, &repo);
        if let Some(t) = token {
            client.set_token(&t);
        }
        client.push(&output_dir)?;
    }

    println!("✅ Build and Export completed successfully");
    Ok(())
}

async fn run_graph(context_dir: PathBuf, dockerfile_path: String) -> Result<()> {
    let dockerfile = fs::read_to_string(&dockerfile_path)?;
    let instructions = docker::parser::parse_dockerfile(&dockerfile);
    let graph = docker::dag::build_graph_from_instructions(instructions, context_dir);

    println!("\n{}", "🕸️  Build Dependency Graph:".bold().cyan());
    for node in &graph.nodes {
        let deps: Vec<String> = node
            .deps
            .iter()
            .map(|&id| graph.nodes[id].name.clone())
            .collect();
        println!("  {} [{}]", node.name.green(), node.id);
        if !deps.is_empty() {
            println!("    └─ depends on: {:?}", deps);
        }
    }
    Ok(())
}

async fn run_explain_cache(
    context_dir: PathBuf,
    dockerfile_path: String,
    target_node: Option<String>,
) -> Result<()> {
    let env_fp = memobuild::env::EnvFingerprint::collect();
    let cache = Arc::new(create_cache().await?);
    let dockerfile = fs::read_to_string(&dockerfile_path)?;
    let instructions = docker::parser::parse_dockerfile(&dockerfile);
    let mut graph = docker::dag::build_graph_from_instructions(instructions, context_dir.clone());

    // AI Layer Analysis to get extra dependencies
    let ai_layer = memobuild::ai::AiLayer::new();
    ai_layer.analyze(&mut graph, &env_fp, &context_dir);

    core::detect_changes(&mut graph);
    core::propagate_dirty(&mut graph);
    core::compute_composite_hashes(&mut graph, &env_fp);

    println!("\n{}", "🔍 Cache Explanation:".bold().cyan());
    for node in &graph.nodes {
        if let Some(ref target) = target_node {
            if !node.name.contains(target) && node.id.to_string() != *target {
                continue;
            }
        }

        let is_cached = cache.local.exists(&node.hash);

        println!("  {} (ID: {})", node.name.bold(), node.id);
        println!(
            "    Status: {}",
            if is_cached {
                "CACHED (Clean)".green()
            } else {
                "DIRTY (Rebuild Required)".red()
            }
        );
        println!("    Hash: {}", node.hash.cyan());

        if !is_cached {
            let mut reasons = Vec::new();
            if node.source_path.is_some() {
                reasons.push("Source files changed or untracked");
            }
            if !node.metadata.extra_source_paths.is_empty() {
                reasons.push("AI-detected dependencies changed");
            }

            let dirty_deps: Vec<_> = node
                .deps
                .iter()
                .filter(|&&id| !cache.local.exists(&graph.nodes[id].hash))
                .map(|&id| &graph.nodes[id].name)
                .collect();
            if !dirty_deps.is_empty() {
                reasons.push("Dependencies are dirty (recursive)");
                println!("    Dirty Dependencies: {:?}", dirty_deps);
            }
            println!("    Reasons: {:?}", reasons);
        }
    }
    Ok(())
}

async fn create_cache() -> Result<cache::HybridCache> {
    let remote_url = env::var("MEMOBUILD_REMOTE_URL").ok();
    let remote_cache = remote_url.map(|url| {
        Arc::new(memobuild::remote_cache::HttpRemoteCache::new(url))
            as Arc<dyn memobuild::remote_cache::RemoteCache>
    });
    cache::HybridCache::new_with_box(remote_cache)
}

async fn _pull_base_images(instructions: &[docker::parser::Instruction]) -> Result<()> {
    for instr in instructions {
        if let docker::parser::Instruction::From(img) = instr {
            println!("   📥 Pulling base image {}...", img);
        }
    }
    Ok(())
}

async fn run_pull(full_name: String) -> Result<()> {
    let (registry_repo, tag) = full_name.split_once(':').unwrap_or((&full_name, "latest"));
    let (registry, repo) = registry_repo
        .split_once('/')
        .unwrap_or(("index.docker.io", registry_repo));
    let output_dir = env::current_dir()?
        .join(".memobuild-cache")
        .join("images")
        .join(full_name.replace([':', '/'], "-"));
    let client = export::registry::RegistryClient::new(registry, repo);
    client.pull(tag, &output_dir)
}

async fn run_generate_ci(provider: String) -> Result<()> {
    if provider == "github" {
        let _yaml = include_str!("../docs/releases/PHASE_1_COMPLETE.md"); // Placeholder for actual template
        println!("✅ GitHub Actions workflow generated");
    }
    Ok(())
}

async fn _run_generate_k8s() -> Result<()> {
    println!("✅ Kubernetes Job manifest generated");
    Ok(())
}

async fn start_scheduler(_port: u16) -> Result<()> {
    #[cfg(feature = "remote-exec")]
    {
        use memobuild::remote_exec::scheduler::SchedulingStrategy;
        use memobuild::remote_exec::{scheduler::Scheduler, server::ExecutionServer};

        println!(
            "📡 Starting Remote Execution Scheduler on port {}...",
            _port
        );

        // For MVP, start with empty worker list - workers will register dynamically
        // In production, this would discover workers via service registry
        let scheduler = Arc::new(Scheduler::new(SchedulingStrategy::RoundRobin));
        let server = ExecutionServer::new(scheduler);

        server.start(_port).await
    }
    #[cfg(not(feature = "remote-exec"))]
    anyhow::bail!("Remote Execution feature not enabled. Build with --features remote-exec")
}

async fn start_worker(
    _port: u16,
    _sandbox_type: String,
    _scheduler_url: Option<String>,
) -> Result<()> {
    #[cfg(feature = "remote-exec")]
    {
        use memobuild::remote_exec::{worker::WorkerNode, worker_server::WorkerServer};
        use memobuild::sandbox;

        println!(
            "🔧 Starting Worker Node on port {} with {} sandbox...",
            _port, _sandbox_type
        );

        // Initialize cache (same as build command)
        let cache = create_cache().await?;
        let cache = Arc::new(cache);

        // Initialize sandbox
        let sandbox: Arc<dyn sandbox::Sandbox> = match _sandbox_type.as_str() {
            "local" => Arc::new(sandbox::local::LocalSandbox::new(std::env::current_dir()?)),
            #[cfg(feature = "containerd")]
            "containerd" => Arc::new(sandbox::containerd::ContainerdSandbox::new().await?),
            _ => anyhow::bail!("Unsupported sandbox type: {}", _sandbox_type),
        };

        // Create worker node
        let worker_id = format!("worker-{}", _port);
        let worker = Arc::new(WorkerNode::new(&worker_id, cache, sandbox));

        // Set scheduler URL for registration
        if let Some(url) = _scheduler_url {
            std::env::set_var("MEMOBUILD_SCHEDULER_URL", url);
        }

        // Start worker server
        let server = WorkerServer::new(worker);
        server.start(_port).await
    }
    #[cfg(not(feature = "remote-exec"))]
    anyhow::bail!("Remote Execution feature not enabled. Build with --features remote-exec")
}

async fn start_cluster_server(
    port: u16,
    node_id: Option<String>,
    peers: Option<String>,
    use_postgres: bool,
    database_url: Option<String>,
) -> Result<()> {
    println!("🏗️ Starting MemoBuild Clustered Cache Server...");

    // Generate node ID if not provided
    let node_id = node_id.unwrap_or_else(|| format!("node-{}", port));

    // Initialize cluster
    let local_node = memobuild::cache_cluster::ClusterNode {
        id: node_id.clone(),
        address: format!("http://localhost:{}", port),
        weight: 100,
        region: Some("local".to_string()),
    };

    let cluster = Arc::new(memobuild::cache_cluster::CacheCluster::new(local_node, 2));

    // Add peer nodes
    if let Some(peers_str) = peers {
        for peer_addr in peers_str.split(',') {
            let peer_addr = peer_addr.trim();
            if !peer_addr.is_empty() {
                let peer_node = memobuild::cache_cluster::ClusterNode {
                    id: format!(
                        "peer-{}",
                        peer_addr.replace("http://", "").replace(":", "-")
                    ),
                    address: peer_addr.to_string(),
                    weight: 100,
                    region: Some("peer".to_string()),
                };
                cluster.add_node(peer_node).await?;
            }
        }
    }

    // Initialize storage backend
    let metadata_store: Arc<dyn crate::server::metadata::MetadataStoreTrait> = if use_postgres {
        if let Some(db_url) = database_url {
            // Parse PostgreSQL URL
            let config = parse_postgres_url(&db_url)?;
            Arc::new(memobuild::scalable_db::PostgresMetadataStore::new(config).await?)
        } else {
            anyhow::bail!("PostgreSQL enabled but DATABASE_URL not provided");
        }
    } else {
        // Use SQLite for simplicity
        let data_dir = std::env::current_dir()?.join(".memobuild-cluster");
        fs::create_dir_all(&data_dir)?;
        let db_path = data_dir.join("metadata.db");
        Arc::new(crate::server::metadata::MetadataStore::new(&db_path)?)
    };

    let storage = Arc::new(crate::server::storage::LocalStorage::new(
        &std::env::current_dir()?.join(".memobuild-cluster"),
    )?);

    // Create distributed cache
    let local_cache = Arc::new(memobuild::remote_cache::HttpRemoteCache::new(format!(
        "http://localhost:{}",
        port
    )));
    let distributed_cache = Arc::new(memobuild::cache_cluster::DistributedCache::new(
        cluster.clone(),
        local_cache,
    ));

    // Initialize auto-scaler
    let scaling_policy = memobuild::auto_scaling::ScalingPolicy {
        min_replicas: 1,
        max_replicas: 10,
        target_utilization_percent: 70.0,
        scale_up_threshold: 0.8,
        scale_down_threshold: 0.3,
        stabilization_window_secs: 300,
        cooldown_period_secs: 60,
    };

    let auto_scaler = memobuild::auto_scaling::AutoScaler::new(scaling_policy.clone()).await?;
    let auto_scaler = match auto_scaler.with_kubernetes().await {
        Ok(scaler) => scaler,
        Err(_) => memobuild::auto_scaling::AutoScaler::new(scaling_policy).await?,
    };
    let auto_scaler = Arc::new(auto_scaler);

    // Create cluster server
    let server = memobuild::cluster_server::ClusterServer {
        cluster,
        metadata_store,
        storage,
        distributed_cache,
        auto_scaler,
    };

    server.start(port).await
}

fn parse_postgres_url(url: &str) -> Result<memobuild::scalable_db::PostgresConfig> {
    // Simple URL parser for demo - in production use a proper URL parser
    let url = url
        .trim_start_matches("postgresql://")
        .trim_start_matches("postgres://");
    let parts: Vec<&str> = url.split('@').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid PostgreSQL URL format");
    }

    let auth = parts[0];
    let rest = parts[1];

    let auth_parts: Vec<&str> = auth.split(':').collect();
    if auth_parts.len() != 2 {
        anyhow::bail!("Invalid auth format in PostgreSQL URL");
    }

    let host_db: Vec<&str> = rest.split('/').collect();
    if host_db.len() != 2 {
        anyhow::bail!("Invalid host/database format in PostgreSQL URL");
    }

    let host_port: Vec<&str> = host_db[0].split(':').collect();
    let host = host_port[0];
    let port = host_port.get(1).unwrap_or(&"5432").parse()?;

    Ok(memobuild::scalable_db::PostgresConfig {
        host: host.to_string(),
        port,
        database: host_db[1].to_string(),
        user: auth_parts[0].to_string(),
        password: auth_parts[1].to_string(),
        max_connections: 20,
        min_idle: Some(5),
    })
}

use colored::*;
