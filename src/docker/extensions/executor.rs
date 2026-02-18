use crate::graph::NodeKind;

pub fn execute_extended_node(node: &NodeKind) {
    match node {
        NodeKind::RunExtend {
            command,
            parallelizable: _,
        } => {
            println!("⚡ Executing extended RUN: {}", command);
            // This would normally delegate to sandbox or specialized logic
        }
        NodeKind::CopyExtend { src, dst, tags: _ } => {
            println!(
                "⚡ Executing extended COPY: {} -> {}",
                src.display(),
                dst.display()
            );
            // This would trigger copy and cache update
        }
        NodeKind::CustomHook { hook_name, .. } => {
            println!("⚡ Running custom hook: {}", hook_name);
        }
        _ => {}
    }
}
