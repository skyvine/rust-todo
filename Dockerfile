FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN apt-get update && apt-get install -y clang lld
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim AS runner
WORKDIR /app
COPY --from=builder /app/target/release/rust-todo /usr/local/bin/rust-todo
RUN apt-get update && apt-get install -y libpq5
ENTRYPOINT ["/usr/local/bin/rust-todo", "--ip-address", "0.0.0.0"]
