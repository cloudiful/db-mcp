# db-mcp

Rust MCP database server built on `cloudiful-server` + `axum`.

## Run

```bash
cargo run -- -config ./dbmcp.example.json
```

Or with env:

```bash
export DBMCP_TRANSPORT=http
export DBMCP_HTTP_ADDR=:8080
export DBMCP_CONFIG=./dbmcp.example.json
export DBMCP_POSTGRES_DSN='postgres://reader:reader@localhost:5432/app?sslmode=disable'
cargo run
```

## Endpoints

- `/mcp`
- `/healthz`
- `/readyz`

## Notes

- `execute_query` only allows `SELECT` and `WITH`
- PostgreSQL and SQLite use `sqlx`
- Oracle is kept behind the same interface, but not enabled in this build
