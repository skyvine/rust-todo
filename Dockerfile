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
RUN apt-get update && apt-get install -y curl libpq5 xz-utils
RUN curl --proto '=https' --tlsv1.2 -LsSf https://github.com/diesel-rs/diesel/releases/latest/download/diesel_cli-installer.sh | sh
WORKDIR /app
COPY --from=builder /app/target/release/rust-todo /app/rust-todo
COPY --from=builder /app/scripts/run_from_container.sh /app/run-rust-todo
COPY --from=builder /app/diesel.toml /app/
# Manually specify each migration until --parents is stabilized
COPY --from=builder /app/migrations/00000000000000_diesel_initial_setup /app/migrations/00000000000000_diesel_initial_setup
COPY --from=builder /app/migrations/2025-06-23-215623_create_users /app/migrations/2025-06-23-215623_create_users
COPY --from=builder /app/migrations/2025-07-06-170408_make-usernames-unique /app/migrations/2025-07-06-170408_make-usernames-unique
COPY --from=builder /app/migrations/2025-07-06-170925_create-auth-keys /app/migrations/2025-07-06-170925_create-auth-keys
COPY --from=builder /app/migrations/2025-07-12-171740_enlarge-users-varchars /app/migrations/2025-07-12-171740_enlarge-users-varchars
ENTRYPOINT ["/app/run-rust-todo", "--ip-address", "0.0.0.0"]
