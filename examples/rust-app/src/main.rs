#[tokio::main]
async fn main() {
    println!("Hello from MemoBuild cached Rust app!");
    println!("Simulating a long build process...");
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    println!("Done!");
}
