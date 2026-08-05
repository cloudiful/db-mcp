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
- HTTP serves the MCP Streamable HTTP endpoint at `/mcp`, plus `/healthz` and `/readyz`; optional bearer auth via config
- There is no implicit default database selection; every MCP tool that touches a database must receive an explicit `connection` name
- Call `list_connections` or read `db://connections` first, then copy one of the returned names into later tool calls
- PostgreSQL and Oracle connections default to `readOnly: true` and run queries inside read-only transactions
- Set `readOnly: false` per connection to allow write-capable transactions
- `execute_query` accepts one single-statement query, including multiline SQL and a trailing `;`; multi-statement SQL scripts are not supported
- Use `execute_queries` for 1 to 8 sequential read-only queries; each query keeps its own result and the batch stops at the first failure
- Batch queries require PostgreSQL/Oracle `readOnly: true`; SQLite additionally requires a read-only DSN such as `mode=ro`
- SQLite does not support transaction-level read-only enforcement here, so single-query writes remain controlled by the DSN and database permissions
- PostgreSQL and SQLite use `sqlx`
- Oracle is kept behind the same interface, but not enabled in this build
