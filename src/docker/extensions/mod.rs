use crate::graph::NodeKind;

pub mod executor;
pub mod parser;
// pub mod node; // We use graph::NodeKind for now to avoid duplication

pub trait DockerExtension: Send + Sync {
    fn name(&self) -> &str;
    fn execute(&self, node: &NodeKind);
}
