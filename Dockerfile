# syntax=docker/dockerfile:1.7

FROM dockerhub.cloud1ful.com/library/rust:1.89-bookworm AS builder
WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked

FROM dockerhub.cloud1ful.com/library/debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /src/target/release/dbmcp /app/dbmcp

EXPOSE 8080
ENTRYPOINT ["/app/dbmcp"]
CMD ["--transport", "http", "--http-addr", ":8080"]
