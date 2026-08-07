# db-mcp

Rust MCP database server. Built on the official Rust MCP SDK (`rmcp`, from the modelcontextprotocol/rust-sdk project) with an embedded `axum` HTTP server; uses `sqlx` for PostgreSQL and SQLite.

## Run

```bash
cargo run -- -config ./dbmcp.example.json
```

Or with env:

```bash
export DBMCP_TRANSPORT=http
export DBMCP_HTTP_ADDR=0.0.0.0:8080
export DBMCP_HTTP_AUTH_MODE=bearer
export DBMCP_HTTP_AUTH_TOKEN='change-me'
export DBMCP_CONFIG=./dbmcp.example.json
export DBMCP_POSTGRES_DSN='postgres://reader:reader@localhost:5432/app?sslmode=disable'
export DBMCP_POSTGRES_READ_ONLY=true
cargo run
```

## Endpoints

- `/mcp`
- `/healthz`
- `/readyz`

## Releases

- Version tags publish `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` binary archives to GitHub Releases
- The container image is published to `ghcr.io/<owner>/<repository>` with `linux/amd64` and `linux/arm64` support

## Notes

- Transport modes: `stdio`, `http`, or `both` (via config or `DBMCP_TRANSPORT`)
- HTTP serves the MCP Streamable HTTP endpoint at `/mcp`, plus `/healthz` and `/readyz`; bearer auth is required by default whenever HTTP binds a non-loopback address (`DBMCP_HTTP_AUTH_MODE=bearer` + `DBMCP_HTTP_AUTH_TOKEN`). An unknown `DBMCP_HTTP_AUTH_MODE` value is a startup error
- CORS is off by default; opt in per origin with `httpCorsAllowedOrigins` or the comma-separated `DBMCP_HTTP_CORS_ALLOWED_ORIGINS` env var
- There is no implicit default database selection; every MCP tool that touches a database must receive an explicit `connection` name
- Call `list_connections` or read `db://connections` first, then copy one of the returned names into later tool calls; health state is refreshed in the background (every 30s) while HTTP is serving, and `list_connections`/`/readyz` return the last cached probe unless `refresh: true` is passed or `diagnose_connection` is used for an on-demand deep check
- PostgreSQL and Oracle connections default to `readOnly: true` and run queries inside read-only transactions
- Set `readOnly: false` per connection to allow write-capable transactions
- `execute_query` accepts one single-statement query, including multiline SQL and a trailing `;`; multi-statement SQL scripts are not supported
- Use `execute_queries` for 1 to 8 sequential read-only queries; each query keeps its own result and the batch stops at the first failure
- Batch queries require PostgreSQL/Oracle `readOnly: true`; SQLite additionally requires a read-only DSN such as `mode=ro`
- SQLite does not support transaction-level read-only enforcement here, so single-query writes remain controlled by the DSN and database permissions; a `readOnly: true` connection with a writable DSN logs a startup warning
- PostgreSQL and SQLite use `sqlx`; Oracle is enabled through `oracle-rs` and is configured per connection the same way
- Integration tests for PostgreSQL and Oracle are skipped unless `DBMCP_TEST_POSTGRES_DSN` or the `DBMCP_TEST_ORACLE_*` vars are set
