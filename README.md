# db-mcp

Rust MCP database server built on `cloudiful-server` + `axum`.

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

## Notes

- PostgreSQL and Oracle connections default to `readOnly: true` and run queries inside read-only transactions
- Set `readOnly: false` per connection to allow write-capable transactions
- SQLite does not support transaction-level read-only enforcement here; use a read-only DSN such as `mode=ro` if needed
- PostgreSQL and SQLite use `sqlx`
- Oracle is kept behind the same interface, but not enabled in this build
