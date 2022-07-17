# Extracts dependencies so we provid from layer caching
# https://www.lpalmieri.com/posts/fast-rust-docker-builds/
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
# Creates dependency layer
RUN cargo chef cook --release --recipe-path recipe.json
# Copy source afterwards so it doesn't affect layer dependencies
COPY . .
# Compile source (dependencies are already build)
RUN cargo build --release --bin kitmatheinfo-bot

FROM ubuntu:latest AS runtime
ENV RUST_LOG="info"
COPY --from=builder /app/target/release/kitmatheinfo-bot /usr/bin/kitmatheinfo-bot
WORKDIR data
ENTRYPOINT ["/usr/bin/kitmatheinfo-bot"]
CMD ["config.toml"]
