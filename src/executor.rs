use crate::graph::BuildGraph;
use crate::cache::HybridCache;
use crate::remote_cache::RemoteCache;
use anyhow::Result;

use rayon::prelude::*;
use std::sync::Arc;

pub fn execute_graph<R: RemoteCache + 'static>(graph: &mut BuildGraph, cache: Arc<HybridCache<R>>) -> Result<()> {
    let levels = graph.levels();

    for (level_idx, level) in levels.iter().enumerate() {
        if level.is_empty() { continue; }
        println!("🚀 Executing level {}: {} nodes", level_idx, level.len());

        // Process level in parallel
        let results: Vec<(usize, bool, bool)> = level.par_iter().map(|&node_id| {
            let node = &graph.nodes[node_id];
            let node_hash = node.hash.clone();

            // 1. Check cache
            match cache.get_artifact(&node_hash) {
                Ok(Some(_data)) => {
                    println!("⚡ Cache HIT: {} [{}]", node.name, &node_hash[..8]);
                    return (node_id, false, true);
                }
                Err(e) => eprintln!("⚠️ Cache error for {}: {}", node.name, e),
                _ => {}
            }

            // 2. Build if dirty
            if node.dirty {
                println!("🔧 Rebuilding node: {}...", node.name);
                let artifact_data = format!("artifact for {}: {}", node.name, node.content).into_bytes();
                
                if let Err(e) = cache.put_artifact(&node_hash, &artifact_data) {
                    eprintln!("⚠️ Cache put error for {}: {}", node.name, e);
                }
                (node_id, false, false)
            } else {
                (node_id, false, false)
            }
        }).collect();

        // Update graph status
        for (node_id, dirty, cache_hit) in results {
            graph.nodes[node_id].dirty = dirty;
            graph.nodes[node_id].cache_hit = cache_hit;
        }
    }
    
    Ok(())
}
