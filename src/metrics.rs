//! Prometheus metrics for MemoBuild
//!
//! This module provides global metrics collection for monitoring cache hits,
//! build performance, cluster health, and garbage collection operations.

use std::sync::Arc;
use tokio::sync::RwLock;

/// Global metrics registry (simplified implementation)
pub struct MetricsRegistry {
    cache_hits: u64,
    cache_misses: u64,
    cluster_nodes: i64,
    replication_lag: f64,
    gc_deleted: u64,
    active_builds: i64,
    error_count: u64,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self {
            cache_hits: 0,
            cache_misses: 0,
            cluster_nodes: 0,
            replication_lag: 0.0,
            gc_deleted: 0,
            active_builds: 0,
            error_count: 0,
        }
    }

    #[allow(dead_code)]
    pub fn inc_cache_hits(&mut self) {
        self.cache_hits += 1;
    }

    #[allow(dead_code)]
    pub fn inc_cache_misses(&mut self) {
        self.cache_misses += 1;
    }

    #[allow(dead_code)]
    pub fn set_cluster_nodes(&mut self, count: u64) {
        self.cluster_nodes = count as i64;
    }

    #[allow(dead_code)]
    pub fn set_replication_lag(&mut self, lag_secs: f64) {
        self.replication_lag = lag_secs;
    }

    #[allow(dead_code)]
    pub fn inc_gc_deleted(&mut self) {
        self.gc_deleted += 1;
    }

    #[allow(dead_code)]
    pub fn set_active_builds(&mut self, count: i64) {
        self.active_builds = count;
    }

    #[allow(dead_code)]
    pub fn inc_errors(&mut self) {
        self.error_count += 1;
    }

    pub fn encode(&self) -> String {
        let mut output = String::new();
        output.push_str("# HELP memobuild_cache_hits_total Total cache hits\n");
        output.push_str("# TYPE memobuild_cache_hits_total counter\n");
        output.push_str(&format!(
            "memobuild_cache_hits_total {}\n\n",
            self.cache_hits
        ));

        output.push_str("# HELP memobuild_cache_misses_total Total cache misses\n");
        output.push_str("# TYPE memobuild_cache_misses_total counter\n");
        output.push_str(&format!(
            "memobuild_cache_misses_total {}\n\n",
            self.cache_misses
        ));

        output.push_str("# HELP memobuild_cluster_nodes_total Number of cluster nodes\n");
        output.push_str("# TYPE memobuild_cluster_nodes_total gauge\n");
        output.push_str(&format!(
            "memobuild_cluster_nodes_total {}\n\n",
            self.cluster_nodes
        ));

        output.push_str("# HELP memobuild_replication_lag_seconds Replication lag in seconds\n");
        output.push_str("# TYPE memobuild_replication_lag_seconds gauge\n");
        output.push_str(&format!(
            "memobuild_replication_lag_seconds {}\n\n",
            self.replication_lag
        ));

        output.push_str("# HELP memobuild_gc_deleted_total Total artifacts deleted by GC\n");
        output.push_str("# TYPE memobuild_gc_deleted_total counter\n");
        output.push_str(&format!(
            "memobuild_gc_deleted_total {}\n\n",
            self.gc_deleted
        ));

        output.push_str("# HELP memobuild_active_builds Number of active builds\n");
        output.push_str("# TYPE memobuild_active_builds gauge\n");
        output.push_str(&format!(
            "memobuild_active_builds {}\n\n",
            self.active_builds
        ));

        output.push_str("# HELP memobuild_errors_total Total errors\n");
        output.push_str("# TYPE memobuild_errors_total counter\n");
        output.push_str(&format!("memobuild_errors_total {}\n", self.error_count));

        output
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedMetrics = Arc<RwLock<MetricsRegistry>>;

pub fn metrics_registry() -> SharedMetrics {
    Arc::new(RwLock::new(MetricsRegistry::new()))
}
