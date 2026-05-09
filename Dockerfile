FROM gitea.cloud1ful.com/docker/rust-ci AS chef
WORKDIR /app

RUN cargo install cargo-chef

COPY .cargo ./.cargo

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    cargo build --release --locked --bin dbmcp

FROM dockerhub.cloud1ful.com/library/debian:trixie-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/dbmcp /usr/local/bin/dbmcp

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/dbmcp"]
CMD ["--transport", "http", "--http-addr", ":8080"]
