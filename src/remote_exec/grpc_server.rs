//! gRPC Server for Remote Execution API
//!
//! This module implements the gRPC services defined in the proto file.

pub mod execution_server;
pub mod reapi;

use std::net::SocketAddr;
use tonic::transport::Server;

pub async fn start_grpc_server(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    
    println!("Starting gRPC server on {}", addr);
    
    // In production, would add the actual services
    // let execution_service = execution_server::ExecutionServiceImpl::new();
    // let cache_service = cache_server::CacheServiceImpl::new();
    
    Server::builder()
        .add_service(
            tonic::reflection::server::Builder::new()
                .register_encoded_file_descriptor_set(EXECUTION_PROTO_DESCRIPTOR)
                .build()
                .unwrap()
        )
        .serve(addr)
        .await?;
    
    Ok(())
}

// Generated protobuf descriptor (would be generated from proto file)
static EXECUTION_PROTO_DESCRIPTOR: &[u8] = b"";