FROM rust:1.75-slim

WORKDIR /app

COPY . .

# Building a Rust app is usually slow, making it a perfect candidate for caching
RUN cargo build --release

CMD ["./target/release/rust-example"]
