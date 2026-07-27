FROM debian:trixie-slim AS app-runtime

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt/lists,sharing=locked \
    apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --chmod=755 ci-image-input/bin/dbmcp /usr/local/bin/dbmcp

EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/dbmcp"]
CMD ["--transport", "http", "--http-addr", "0.0.0.0:8080"]
