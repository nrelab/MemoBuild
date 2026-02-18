use crate::cache::HybridCache;
use crate::graph::BuildGraph;
use crate::remote_cache::RemoteCache;
use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;

/// Incremental executor that supports parallel execution and selective rebuilds
pub struct IncrementalExecutor<R: RemoteCache + 'static> {
    cache: Arc<HybridCache<R>>,
    execution_stats: ExecutionStats,
    observer: Option<Arc<dyn crate::dashboard::BuildObserver>>,
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

impl<R: RemoteCache + 'static> IncrementalExecutor<R> {
    pub fn new(cache: Arc<HybridCache<R>>) -> Self {
        Self {
            cache,
            execution_stats: ExecutionStats::default(),
            observer: None,
        }
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
            let node = &graph.nodes[node_id];
            let name = node.name.clone();
            let hash = node.hash.clone();
            let dirty = node.dirty;
            let kind = node.kind.clone();
            let cache = self.cache.clone();
            let observer = self.observer.clone();

            futures.push(async move {
                if let Some(ref obs) = observer {
                    obs.on_event(crate::dashboard::BuildEvent::NodeStarted {
                        node_id,
                        name: name.clone(),
                    });
                }
                let start_time = Instant::now();
                let result = Self::execute_node_logic(cache, &name, &hash, dirty, &kind).await;
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
                &node.name,
                &node.hash,
                node.dirty,
                &node.kind,
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

    async fn execute_node_logic(
        cache: Arc<HybridCache<R>>,
        name: &str,
        hash: &str,
        dirty: bool,
        kind: &crate::graph::NodeKind,
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
            let artifact_data = Self::build_node_artifact_static(kind)?;

            if let Err(e) = cache.put_artifact(hash, &artifact_data).await {
                eprintln!("⚠️ Cache put error for {}: {}", name, e);
            }
            Ok((false, false))
        } else {
            // Node is clean but not in cache - rebuild it
            println!("🔨 Building clean node: {}...", name);
            let artifact_data = Self::build_node_artifact_static(kind)?;

            if let Err(e) = cache.put_artifact(hash, &artifact_data).await {
                eprintln!("⚠️ Cache put error for {}: {}", name, e);
            }
            Ok((false, false))
        }
    }

    fn build_node_artifact_static(kind: &crate::graph::NodeKind) -> Result<Vec<u8>> {
        let artifact_content = match kind {
            crate::graph::NodeKind::From => "FROM artifact".to_string(),
            crate::graph::NodeKind::Run => "RUN artifact".to_string(),
            crate::graph::NodeKind::Copy { src, dst } => {
                format!("COPY artifact: {} -> {}", src.display(), dst.display())
            }
            crate::graph::NodeKind::Env => "ENV artifact".to_string(),
            crate::graph::NodeKind::Workdir => "WORKDIR artifact".to_string(),
            crate::graph::NodeKind::Cmd => "CMD artifact".to_string(),
            crate::graph::NodeKind::Git { url, target } => {
                format!("GIT artifact: {} -> {}", url, target.display())
            }
            crate::graph::NodeKind::Other => "OTHER artifact".to_string(),
        };

        Ok(artifact_content.into_bytes())
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
pub async fn execute_graph<R: RemoteCache + 'static>(
    graph: &mut BuildGraph,
    cache: Arc<HybridCache<R>>,
    observer: Option<Arc<dyn crate::dashboard::BuildObserver>>,
) -> Result<()> {
    let mut executor = IncrementalExecutor::new(cache);
    if let Some(obs) = observer {
        executor = executor.with_observer(obs);
    }
    executor.execute(graph).await?;
    Ok(())
}
