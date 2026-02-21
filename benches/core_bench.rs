use criterion::{criterion_group, criterion_main, Criterion};
use memobuild::hasher::walker;
use std::fs;
use tempfile::tempdir;

fn bench_hashing(c: &mut Criterion) {
    let dir = tempdir().unwrap();
    let path = dir.path();
    
    // Create a mock directory structure to hash
    for i in 0..100 {
        fs::write(path.join(format!("file_{}.txt", i)), format!("content {}", i)).unwrap();
    }

    c.bench_function("directory hasher", |b| b.iter(|| {
        let _ = walker::fast_hash_directory(path, &[]);
    }));
}

criterion_group!(benches, bench_hashing);
criterion_main!(benches);
