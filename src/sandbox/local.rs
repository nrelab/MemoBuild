use anyhow::Result;
use async_trait::async_trait;
use crate::graph::Node;
use crate::sandbox::{Sandbox, SandboxEnv, ExecResult};
use std::process::Command;

pub struct LocalSandbox;

#[async_trait]
impl Sandbox for LocalSandbox {
    async fn prepare(&self, node: &Node) -> Result<SandboxEnv> {
        // Local sandbox uses the current working directory but can isolate via temp dirs
        let workspace_dir = std::env::current_dir()?;
        
        Ok(SandboxEnv {
            workspace_dir,
            env_vars: node.env.clone(),
        })
    }

    async fn execute(&self, env: &SandboxEnv, node: &Node) -> Result<ExecResult> {
        let cmd = match &node.kind {
            crate::graph::NodeKind::Run => &node.content,
            _ => {
                // For non-RUN nodes, we simulate success and return a metadata-based artifact
                return Ok(ExecResult {
                    exit_code: 0,
                    stdout: format!("Artifact for {}", node.name).into_bytes(),
                    stderr: Vec::new(),
                });
            }
        };

        let output = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .arg("/C")
                .arg(cmd)
                .envs(&env.env_vars)
                .current_dir(&env.workspace_dir)
                .output()?
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .envs(&env.env_vars)
                .current_dir(&env.workspace_dir)
                .output()?
        };

        Ok(ExecResult {
            exit_code: output.status.code().unwrap_or(1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    async fn cleanup(&self, _env: &SandboxEnv) -> Result<()> {
        Ok(())
    }
}
