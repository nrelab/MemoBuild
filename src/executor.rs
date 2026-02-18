use crate::cache::HybridCache;
use crate::graph::BuildGraph;
use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;

/// Incremental executor that supports parallel execution and selective rebuilds
pub struct IncrementalExecutor {
    cache: Arc<HybridCache>,
    execution_stats: ExecutionStats,
    observer: Option<Arc<dyn crate::dashboard::BuildObserver>>,
    reproducible: bool,
    sandbox: Arc<dyn crate::sandbox::Sandbox>,
    remote_executor: Option<Arc<dyn crate::remote_exec::RemoteExecutor>>,
}

#[derive(Debug, Default, Clone)]
pub struct ExecutionStats {
    pub total_nodes: usize,
    pub executed_nodes: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub parallel_levels: usize,
    pub total_execution_time_ms: u64,
}

impl IncrementalExecutor {
    pub fn new(cache: Arc<HybridCache>) -> Self {
        Self {
            cache,
            execution_stats: ExecutionStats::default(),
            observer: None,
            reproducible: false,
            sandbox: Arc::new(crate::sandbox::local::LocalSandbox),
            remote_executor: None,
        }
    }

    pub fn with_remote_executor(
        mut self,
        exec: Arc<dyn crate::remote_exec::RemoteExecutor>,
    ) -> Self {
        self.remote_executor = Some(exec);
        self
    }

    pub fn with_sandbox(mut self, sandbox: Arc<dyn crate::sandbox::Sandbox>) -> Self {
        self.sandbox = sandbox;
        self
    }

    pub fn with_reproducible(mut self, reproducible: bool) -> Self {
        self.reproducible = reproducible;
        self
    }

    pub fn with_observer(mut self, observer: Arc<dyn crate::dashboard::BuildObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Execute the build graph with parallel and incremental capabilities
    pub async fn execute(&mut self, graph: &mut BuildGraph) -> Result<ExecutionStats> {
        let start_time = Instant::now();

        // Reset stats
        self.execution_stats = ExecutionStats::default();
        self.execution_stats.total_nodes = graph.nodes.len();

        // Get execution levels for parallel processing
        let levels = graph.levels();
        self.execution_stats.parallel_levels = levels.len();

        if let Some(ref obs) = self.observer {
            obs.on_event(crate::dashboard::BuildEvent::BuildStarted {
                total_nodes: self.execution_stats.total_nodes,
            });
        }

        println!(
            "🚀 Starting incremental execution with {} levels",
            levels.len()
        );

        for (level_idx, level) in levels.iter().enumerate() {
            if level.is_empty() {
                continue;
            }

            println!(" Executing level {}: {} nodes", level_idx, level.len());

            let (parallel_nodes, sequential_nodes): (Vec<_>, Vec<_>) = level
                .iter()
                .partition(|&&node_id| graph.nodes[node_id].metadata.parallelizable);

            // Execute parallel nodes first
            if !parallel_nodes.is_empty() {
                self.execute_parallel_nodes(graph, &parallel_nodes).await?;
            }

            // Execute sequential nodes
            if !sequential_nodes.is_empty() {
                self.execute_sequential_nodes(graph, &sequential_nodes)
                    .await?;
            }
            // Finalize execute
        }

        self.execution_stats.total_execution_time_ms = start_time.elapsed().as_millis() as u64;

        if let Some(ref obs) = self.observer {
            obs.on_event(crate::dashboard::BuildEvent::BuildCompleted {
                total_duration_ms: self.execution_stats.total_execution_time_ms,
                cache_hits: self.execution_stats.cache_hits,
                executed_nodes: self.execution_stats.executed_nodes,
            });
        }

        self.print_execution_summary();

        Ok(self.execution_stats.clone())
    }

    /// Execute nodes in parallel
    async fn execute_parallel_nodes(
        &mut self,
        graph: &mut BuildGraph,
        node_ids: &[&usize],
    ) -> Result<()> {
        println!("⚡ Executing {} nodes in parallel", node_ids.len());

        let mut futures = Vec::new();

        for &&node_id in node_ids {
            let node = graph.nodes[node_id].clone();
            let name = node.name.clone();
            let hash = node.hash.clone();
            let dirty = node.dirty;
            let kind = node.kind.clone();
            let cache = self.cache.clone();
            let observer = self.observer.clone();
            let sandbox = self.sandbox.clone();
            let remote_executor = self.remote_executor.clone();
            let reproducible = self.reproducible;

            futures.push(async move {
                if let Some(ref obs) = observer {
                    obs.on_event(crate::dashboard::BuildEvent::NodeStarted {
                        node_id,
                        name: name.clone(),
                    });
                }
                let start_time = Instant::now();
                let result = Self::execute_node_logic(
                    cache,
                    node_id,
                    &name,
                    &hash,
                    dirty,
                    &kind,
                    reproducible,
                    sandbox,
                    remote_executor,
                    &node,
                )
                .await;
                let execution_time = start_time.elapsed().as_millis() as u64;

                if let Some(ref obs) = observer {
                    match &result {
                        Ok((_, cache_hit)) => {
                            obs.on_event(crate::dashboard::BuildEvent::NodeCompleted {
                                node_id,
                                name: name.clone(),
                                duration_ms: execution_time,
                                cache_hit: *cache_hit,
                            })
                        }
                        Err(e) => obs.on_event(crate::dashboard::BuildEvent::NodeFailed {
                            node_id,
                            name: name.clone(),
                            error: e.to_string(),
                        }),
                    }
                }
                (node_id, result, execution_time)
            });
        }

        let results = futures::future::join_all(futures).await;

        // Update graph status and stats
        for (node_id, result, execution_time) in results {
            let (dirty, cache_hit) = result?;

            graph.nodes[node_id].dirty = dirty;
            graph.nodes[node_id].cache_hit = cache_hit;
            graph.nodes[node_id].metadata.last_executed = Some(std::time::SystemTime::now());
            graph.nodes[node_id].metadata.execution_time_ms = Some(execution_time);

            if cache_hit {
                self.execution_stats.cache_hits += 1;
            } else {
                self.execution_stats.cache_misses += 1;
                self.execution_stats.executed_nodes += 1;
            }
        }

        Ok(())
    }

    /// Execute nodes sequentially
    async fn execute_sequential_nodes(
        &mut self,
        graph: &mut BuildGraph,
        node_ids: &[&usize],
    ) -> Result<()> {
        println!("🔧 Executing {} nodes sequentially", node_ids.len());

        for &&node_id in node_ids {
            let start_time = Instant::now();
            let node = &graph.nodes[node_id];

            if let Some(ref obs) = self.observer {
                obs.on_event(crate::dashboard::BuildEvent::NodeStarted {
                    node_id,
                    name: node.name.clone(),
                });
            }

            let result = Self::execute_node_logic(
                self.cache.clone(),
                node_id,
                &node.name,
                &node.hash,
                node.dirty,
                &node.kind,
                self.reproducible,
                self.sandbox.clone(),
                self.remote_executor.clone(),
                node,
            )
            .await;

            let execution_time = start_time.elapsed().as_millis() as u64;

            if let Some(ref obs) = self.observer {
                match &result {
                    Ok((_, cache_hit)) => {
                        obs.on_event(crate::dashboard::BuildEvent::NodeCompleted {
                            node_id,
                            name: node.name.clone(),
                            duration_ms: execution_time,
                            cache_hit: *cache_hit,
                        })
                    }
                    Err(e) => obs.on_event(crate::dashboard::BuildEvent::NodeFailed {
                        node_id,
                        name: node.name.clone(),
                        error: e.to_string(),
                    }),
                }
            }

            let (dirty, cache_hit) = result?;

            graph.nodes[node_id].dirty = dirty;
            graph.nodes[node_id].cache_hit = cache_hit;
            graph.nodes[node_id].metadata.last_executed = Some(std::time::SystemTime::now());
            graph.nodes[node_id].metadata.execution_time_ms = Some(execution_time);

            if cache_hit {
                self.execution_stats.cache_hits += 1;
            } else {
                self.execution_stats.cache_misses += 1;
                self.execution_stats.executed_nodes += 1;
            }
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_node_logic(
        cache: Arc<HybridCache>,
        _node_id: usize,
        name: &str,
        hash: &str,
        dirty: bool,
        _kind: &crate::graph::NodeKind,
        reproducible: bool,
        sandbox: Arc<dyn crate::sandbox::Sandbox>,
        remote_executor: Option<Arc<dyn crate::remote_exec::RemoteExecutor>>,
        node: &crate::graph::Node,
    ) -> Result<(bool, bool)> {
        // 1. Check cache first
        match cache.get_artifact(hash).await {
            Ok(Some(_data)) => {
                println!("⚡ Cache HIT: {} [{}]", name, &hash[..8]);
                return Ok((false, true));
            }
            Err(e) => eprintln!("⚠️ Cache error for {}: {}", name, e),
            _ => {}
        }

        // 2. Build if dirty or not cached
        if dirty {
            println!("🔧 Rebuilding node: {}...", name);
        } else {
            println!("🔨 Building clean node (not in cache): {}...", name);
        }

        let mut artifact_data = if let Some(ref remote) = remote_executor {
            println!("📡 [RemoteExec] Dispatching node {} to build farm", name);
            let action = crate::remote_exec::ActionRequest {
                command: vec!["/bin/sh".into(), "-c".into(), node.content.clone()],
                env: node.env.clone(),
                input_root_digest: crate::remote_exec::Digest {
                    hash: hash.to_string(),
                    size_bytes: 0, // Placeholder
                },
                timeout: std::time::Duration::from_secs(600),
                platform_properties: std::collections::HashMap::new(),
                output_files: Vec::new(),
                output_directories: Vec::new(),
            };

            let result = remote.execute(action).await?;
            if result.exit_code != 0 {
                anyhow::bail!(
                    "Remote execution failed with exit code {}: {}",
                    result.exit_code,
                    String::from_utf8_lossy(&result.stderr_raw)
                );
            }
            result.stdout_raw
        } else {
            // Prepare sandbox
            if let crate::graph::NodeKind::RunExtend { command, .. } = &node.kind {
                println!("⚡ Executing extended RUN: {}", command);
            } else if let crate::graph::NodeKind::CopyExtend { src, dst, .. } = &node.kind {
                println!(
                    "⚡ Executing extended COPY: {} -> {}",
                    src.display(),
                    dst.display()
                );
            } else if let crate::graph::NodeKind::CustomHook { hook_name, .. } = &node.kind {
                println!("⚡ Running custom hook: {}", hook_name);
            }

            let env = sandbox.prepare(node).await?;

            // Execute command
            let exec_result = sandbox.execute(&env, node).await?;

            if exec_result.exit_code != 0 {
                anyhow::bail!(
                    "Command failed with exit code {}: {}",
                    exec_result.exit_code,
                    String::from_utf8_lossy(&exec_result.stderr)
                );
            }

            let data = exec_result.stdout;
            sandbox.cleanup(&env).await?;
            data
        };

        if reproducible {
            artifact_data = crate::reproducible::normalize_artifact(artifact_data)?;
        }

        if let Err(e) = cache.put_artifact(hash, &artifact_data).await {
            eprintln!("⚠️ Cache put error for {}: {}", name, e);
        }

        Ok((false, false))
    }

    /// Print execution summary
    fn print_execution_summary(&self) {
        println!("\n📊 Execution Summary:");
        println!("  Total nodes: {}", self.execution_stats.total_nodes);
        println!("  Executed nodes: {}", self.execution_stats.executed_nodes);
        println!("  Cache hits: {}", self.execution_stats.cache_hits);
        println!("  Cache misses: {}", self.execution_stats.cache_misses);
        println!(
            "  Parallel levels: {}",
            self.execution_stats.parallel_levels
        );
        println!(
            "  Total time: {}ms",
            self.execution_stats.total_execution_time_ms
        );

        if self.execution_stats.total_nodes > 0 {
            let cache_hit_rate = (self.execution_stats.cache_hits as f64
                / self.execution_stats.total_nodes as f64)
                * 100.0;
            println!("  Cache hit rate: {:.1}%", cache_hit_rate);
        }
    }
}

/// Legacy function for backward compatibility
pub async fn execute_graph(
    graph: &mut BuildGraph,
    cache: Arc<HybridCache>,
    observer: Option<Arc<dyn crate::dashboard::BuildObserver>>,
    reproducible: bool,
) -> Result<()> {
    let mut executor = IncrementalExecutor::new(cache).with_reproducible(reproducible);
    if let Some(obs) = observer {
        executor = executor.with_observer(obs);
    }
    executor.execute(graph).await?;
    Ok(())
}
