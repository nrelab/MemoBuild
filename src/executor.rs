use crate::cache::HybridCache;
use crate::graph::BuildGraph;
use crate::remote_cache::RemoteCache;
use anyhow::Result;
use rayon::prelude::*;
use std::sync::Arc;
use std::time::Instant;

/// Incremental executor that supports parallel execution and selective rebuilds
pub struct IncrementalExecutor<R: RemoteCache + 'static> {
    cache: Arc<HybridCache<R>>,
    execution_stats: ExecutionStats,
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
        }
    }

    /// Execute the build graph with parallel and incremental capabilities
    pub fn execute(&mut self, graph: &mut BuildGraph) -> Result<ExecutionStats> {
        let start_time = Instant::now();

        // Reset stats
        self.execution_stats = ExecutionStats::default();
        self.execution_stats.total_nodes = graph.nodes.len();

        // Get execution levels for parallel processing
        let levels = graph.levels();
        self.execution_stats.parallel_levels = levels.len();

        println!(
            "🚀 Starting incremental execution with {} levels",
            levels.len()
        );

        for (level_idx, level) in levels.iter().enumerate() {
            if level.is_empty() {
                continue;
            }

            println!("� Executing level {}: {} nodes", level_idx, level.len());

            let (parallel_nodes, sequential_nodes): (Vec<_>, Vec<_>) = level
                .iter()
                .partition(|&&node_id| graph.nodes[node_id].metadata.parallelizable);

            // Execute parallel nodes first
            if !parallel_nodes.is_empty() {
                self.execute_parallel_nodes(graph, &parallel_nodes)?;
            }

            // Execute sequential nodes
            if !sequential_nodes.is_empty() {
                self.execute_sequential_nodes(graph, &sequential_nodes)?;
            }
        }

        self.execution_stats.total_execution_time_ms = start_time.elapsed().as_millis() as u64;
        self.print_execution_summary();

        Ok(self.execution_stats.clone())
    }

    /// Execute nodes in parallel using Rayon
    fn execute_parallel_nodes(
        &mut self,
        graph: &mut BuildGraph,
        node_ids: &[&usize],
    ) -> Result<()> {
        println!("⚡ Executing {} nodes in parallel", node_ids.len());

        let results: Vec<(usize, bool, bool, Option<u64>)> = node_ids
            .par_iter()
            .map(|&&node_id| {
                let start_time = Instant::now();
                let result = self.execute_single_node(graph, node_id);
                let execution_time = start_time.elapsed().as_millis() as u64;

                match result {
                    Ok((dirty, cache_hit)) => (node_id, dirty, cache_hit, Some(execution_time)),
                    Err(e) => {
                        eprintln!("⚠️ Error executing node {}: {}", node_id, e);
                        (node_id, false, false, Some(execution_time))
                    }
                }
            })
            .collect();

        // Update graph status and stats
        for (node_id, dirty, cache_hit, execution_time) in results {
            graph.nodes[node_id].dirty = dirty;
            graph.nodes[node_id].cache_hit = cache_hit;
            graph.nodes[node_id].metadata.last_executed = Some(std::time::SystemTime::now());
            graph.nodes[node_id].metadata.execution_time_ms = execution_time;

            if cache_hit {
                self.execution_stats.cache_hits += 1;
            } else {
                self.execution_stats.cache_misses += 1;
                self.execution_stats.executed_nodes += 1;
            }
        }

        Ok(())
    }

    /// Execute nodes sequentially (for non-parallelizable operations)
    fn execute_sequential_nodes(
        &mut self,
        graph: &mut BuildGraph,
        node_ids: &[&usize],
    ) -> Result<()> {
        println!("🔧 Executing {} nodes sequentially", node_ids.len());

        for &&node_id in node_ids {
            let start_time = Instant::now();

            match self.execute_single_node(graph, node_id) {
                Ok((dirty, cache_hit)) => {
                    let execution_time = start_time.elapsed().as_millis() as u64;

                    graph.nodes[node_id].dirty = dirty;
                    graph.nodes[node_id].cache_hit = cache_hit;
                    graph.nodes[node_id].metadata.last_executed =
                        Some(std::time::SystemTime::now());
                    graph.nodes[node_id].metadata.execution_time_ms = Some(execution_time);

                    if cache_hit {
                        self.execution_stats.cache_hits += 1;
                    } else {
                        self.execution_stats.cache_misses += 1;
                        self.execution_stats.executed_nodes += 1;
                    }
                }
                Err(e) => {
                    eprintln!("⚠️ Error executing sequential node {}: {}", node_id, e);
                }
            }
        }

        Ok(())
    }

    /// Execute a single node with cache checking and building
    fn execute_single_node(&self, graph: &BuildGraph, node_id: usize) -> Result<(bool, bool)> {
        let node = &graph.nodes[node_id];
        let node_hash = node.hash.clone();

        // 1. Check cache first
        match self.cache.get_artifact(&node_hash) {
            Ok(Some(_data)) => {
                println!("⚡ Cache HIT: {} [{}]", node.name, &node_hash[..8]);
                return Ok((false, true));
            }
            Err(e) => eprintln!("⚠️ Cache error for {}: {}", node.name, e),
            _ => {}
        }

        // 2. Build if dirty or not cached
        if node.dirty {
            println!("🔧 Rebuilding node: {}...", node.name);
            let artifact_data = self.build_node_artifact(node)?;

            if let Err(e) = self.cache.put_artifact(&node_hash, &artifact_data) {
                eprintln!("⚠️ Cache put error for {}: {}", node.name, e);
            }
            Ok((false, false))
        } else {
            // Node is clean but not in cache - rebuild it
            println!("🔨 Building clean node: {}...", node.name);
            let artifact_data = self.build_node_artifact(node)?;

            if let Err(e) = self.cache.put_artifact(&node_hash, &artifact_data) {
                eprintln!("⚠️ Cache put error for {}: {}", node.name, e);
            }
            Ok((false, false))
        }
    }

    /// Build artifact for a node (placeholder implementation)
    fn build_node_artifact(&self, node: &crate::graph::Node) -> Result<Vec<u8>> {
        // This is a placeholder - in a real implementation, this would
        // execute the actual Docker operation or build step
        let artifact_content = match &node.kind {
            crate::graph::NodeKind::From => format!("FROM artifact: {}", node.content),
            crate::graph::NodeKind::Run => format!("RUN artifact: {}", node.content),
            crate::graph::NodeKind::Copy { src, dst } => {
                format!("COPY artifact: {} -> {}", src.display(), dst.display())
            }
            crate::graph::NodeKind::Env => format!("ENV artifact: {}", node.content),
            crate::graph::NodeKind::Workdir => format!("WORKDIR artifact: {}", node.content),
            crate::graph::NodeKind::Cmd => format!("CMD artifact: {}", node.content),
            crate::graph::NodeKind::Git { url, target } => {
                format!("GIT artifact: {} -> {}", url, target.display())
            }
            crate::graph::NodeKind::Other => format!("OTHER artifact: {}", node.content),
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
pub fn execute_graph<R: RemoteCache + 'static>(
    graph: &mut BuildGraph,
    cache: Arc<HybridCache<R>>,
) -> Result<()> {
    let mut executor = IncrementalExecutor::new(cache);
    executor.execute(graph)?;
    Ok(())
}
