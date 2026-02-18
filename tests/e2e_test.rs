use memobuild::{cache, server, remote_cache, executor, docker, core};
use std::sync::Arc;
use tempfile::tempdir;
use std::fs;
use std::thread;
use std::time::Duration;

#[test]
fn test_end_to_end_build_with_remote_cache() {
    // 1. Setup temporary directories
    let server_dir = tempdir().expect("Failed to create server temp dir");
    let client_dir = tempdir().expect("Failed to create client temp dir");
    let build_dir = tempdir().expect("Failed to create build temp dir");
    
    let server_path = server_dir.path().to_path_buf();
    let client_path = client_dir.path().to_path_buf();
    let build_path = build_dir.path().to_path_buf();

    // 2. Start Remote Cache Server in background thread
    let port = 9991;
    let server_path_clone = server_path.clone();
    thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            server::start_server(port, server_path_clone, None).await.ok();
        });
    });

    // Wait for server to start
    thread::sleep(Duration::from_millis(1000));

    // 3. Configure Client Environment
    std::env::set_var("MEMOBUILD_CACHE_DIR", client_path.to_str().unwrap());
    
    // 4. Create a dummy project in build_dir
    let dockerfile_path = build_path.join("Dockerfile");
    fs::write(&dockerfile_path, "FROM alpine\nRUN echo 'hello world' > /hello.txt").unwrap();
    
    let original_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&build_path).unwrap();

    // 5. Initialize Cache with Remote Support
    let remote_url = format!("http://127.0.0.1:{}", port);
    let remote = remote_cache::HttpRemoteCache::new(remote_url);
    let cache = Arc::new(cache::HybridCache::new(Some(remote)).unwrap());

    // 6. Run First Build (Populate Cache)
    let dockerfile_content = "FROM alpine\nRUN echo 'hello world' > /hello.txt";
    let instructions = docker::parser::parse_dockerfile(dockerfile_content);
    let mut graph = docker::dag::build_graph_from_instructions(instructions);
    
    core::detect_changes(&mut graph);
    core::propagate_dirty(&mut graph);
    
    executor::execute_graph(&mut graph, cache.clone()).expect("First build failed");

    // 7. Run Second Build (Should be cached)
    // Manually clear local cache to force remote fetch
    fs::remove_dir_all(&client_path).unwrap();
    fs::create_dir_all(&client_path).unwrap();
    
    let instructions2 = docker::parser::parse_dockerfile(dockerfile_content);
    let mut graph2 = docker::dag::build_graph_from_instructions(instructions2);
    
    core::detect_changes(&mut graph2);
    core::propagate_dirty(&mut graph2);
    
    let remote2 = remote_cache::HttpRemoteCache::new(format!("http://127.0.0.1:{}", port));
    let cache2 = Arc::new(cache::HybridCache::new(Some(remote2)).unwrap());
    
    executor::execute_graph(&mut graph2, cache2.clone()).expect("Second build failed");

    // Verify that nodes were fetched from remote (cached should be > 0)
    let _cached_count = graph2.nodes.iter().filter(|n| !n.dirty).count();
    assert!(_cached_count > 0, "Build should have used remote cache items");
    // In this simple test, Alpine might be cached or dirty depending on environment.
    // But the RUN node should definitely be cached if it worked.
    
    // 8. Cleanup
    std::env::set_current_dir(original_cwd).unwrap();
}
