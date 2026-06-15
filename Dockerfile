FROM forgejo.cloud1ful.com/docker/rust-ci:latest AS chef
WORKDIR /app

FROM chef AS planner
# Copy full workspace so cargo-chef sees new crates/modules without Dockerfile churn.
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ENV SCCACHE_DIR=/var/cache/sccache
COPY --from=planner /app/recipe.json recipe.json
RUN --mount=type=cache,id=sccache,target=/var/cache/sccache \
    --mount=type=cache,id=cargo-registry,target=/root/.cargo/registry \
    --mount=type=cache,id=cargo-git,target=/root/.cargo/git \
    cargo chef cook --release --recipe-path recipe.json

COPY . .
RUN --mount=type=cache,id=sccache,target=/var/cache/sccache \
    --mount=type=cache,id=cargo-registry,target=/root/.cargo/registry \
    --mount=type=cache,id=cargo-git,target=/root/.cargo/git \
    cargo build --release --bin dbmcp

FROM dockerhub.cloud1ful.com/library/debian:trixie-slim AS app-runtime

COPY apt-cloud1ful-insecure.conf /etc/apt/apt.conf.d/99apt-cloud1ful-insecure
COPY debian.sources /etc/apt/sources.list.d/debian.sources
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/dbmcp /usr/local/bin/dbmcp

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/dbmcp"]
CMD ["--transport", "http", "--http-addr", "0.0.0.0:8080"]
