# db-mcp

Minimal MCP server for databases, implemented in Go. Oracle, PostgreSQL, and SQLite are supported today, and the service layer stays generic so more drivers can be added without changing the MCP tool surface.

## Features

- MCP tools: `list_connections`, `list_schemas`, `list_tables`, `search_columns`, `list_foreign_keys`, `list_indexes`, `get_table_comments`, `describe_table`, `execute_query`, `explain_query`
- MCP prompts: `explore_database`, `inspect_table`
- MCP resources: `db://connections` and `db://table/{connection}/{schema}/{table}`
- Server-level MCP instructions to steer clients toward safe schema discovery before ad hoc SQL
- Oracle support via a pure Go driver, so it can connect directly from a Go binary without Oracle Instant Client
- PostgreSQL support via `pgx` stdlib
- SQLite support via the pure Go `modernc.org/sqlite` driver
- Supports `stdio`, `http`, and `both` transport modes
- Explicit read-only transaction handling for query execution and metadata reads
- Row, cell, column, and total output truncation to avoid blowing up MCP context

## Config

Use a JSON config file:

```bash
go run ./cmd/dbmcp -config ./dbmcp.example.json
```

Or use environment variables:

```bash
export DBMCP_TRANSPORT=stdio
export DBMCP_DEBUG=true
export DBMCP_ORACLE_JDBC_URL='jdbc:oracle:thin:@unraid.cloud1ful.com:1521:XE'
export DBMCP_ORACLE_USERNAME=readuser
export DBMCP_ORACLE_PASSWORD=readuser
export DBMCP_POSTGRES_DSN='postgres://reader:reader@localhost:5432/app?sslmode=disable'
export DBMCP_SQLITE_DSN='file:./app.db?mode=ro'
go run ./cmd/dbmcp
```

Supported environment variables for connections:

- `DBMCP_DEBUG` enables extra startup diagnostics
- `DBMCP_ORACLE_JDBC_URL`, `DBMCP_ORACLE_USERNAME`, `DBMCP_ORACLE_PASSWORD`, optional `DBMCP_ORACLE_NAME`
- `DBMCP_POSTGRES_DSN`, optional `DBMCP_POSTGRES_NAME`
- `DBMCP_SQLITE_DSN`, optional `DBMCP_SQLITE_NAME`
- `DATABASE_URL` is also accepted as the PostgreSQL DSN fallback
- `SQLITE_URL` is also accepted as the SQLite DSN fallback

For HTTP mode:

```bash
go run ./cmd/dbmcp -config ./dbmcp.example.json -transport http -http-addr :8080
```

The streamable HTTP endpoint defaults to `/mcp`.

## Docker

Build the image with your Nexus mirror:

```bash
docker build -t db-mcp:latest .
```

Run it in HTTP mode:

```bash
docker run --rm -p 8080:8080 \
  -e DBMCP_ORACLE_JDBC_URL='jdbc:oracle:thin:@unraid.cloud1ful.com:1521:XE' \
  -e DBMCP_ORACLE_USERNAME=readuser \
  -e DBMCP_ORACLE_PASSWORD=readuser \
  -e DBMCP_POSTGRES_DSN='postgres://reader:reader@postgres:5432/app?sslmode=disable' \
  -e DBMCP_SQLITE_DSN='file:/data/app.db?mode=ro' \
  db-mcp:latest
```

The container defaults to:

- `-transport http`
- `-http-addr :8080`
- MCP endpoint: `http://<host>:8080/mcp`
- Health endpoint: `http://<host>:8080/healthz`

On startup the server now logs:

- each configured database connection before connect, including a redacted PostgreSQL target
- enabled transport
- HTTP host, port, and MCP path when HTTP is enabled
- each configured database connection and whether it connected successfully

You can also enable debug logging with:

```bash
go run ./cmd/dbmcp -debug=true
```

For SQLite, prefer a read-only DSN such as:

```text
file:./app.db?mode=ro
```

Override startup flags if needed:

```bash
docker run --rm -p 8081:8081 \
  -e DBMCP_ORACLE_JDBC_URL='jdbc:oracle:thin:@unraid.cloud1ful.com:1521:XE' \
  -e DBMCP_ORACLE_USERNAME=readuser \
  -e DBMCP_ORACLE_PASSWORD=readuser \
  db-mcp:latest -transport http -http-addr :8081
```

## Notes

- Prefer the discovery flow `list_connections` / `db://connections` -> `list_schemas` -> `list_tables` / `search_columns` -> `list_foreign_keys` / `list_indexes` / `get_table_comments` / `describe_table` before `execute_query`.
- If the MCP client supports resources, `db://table/{connection}/{schema}/{table}` returns a compact JSON bundle for one table with columns, foreign keys, indexes, and comments.
- Resources are a native MCP concept, not a custom extension in this repo. They are read-oriented endpoints that let the client fetch structured context such as `db://connections` or one table overview without inventing an extra tool call.
- This server's MCP instructions, prompts, tools, and resources are registered in `internal/mcpserver/server.go`. Clients vary in how visibly they surface prompts/resources/instructions, but the server exports them all.
- `execute_query` only accepts `SELECT` and `WITH` statements.
- Any attempt to send multiple statements or `FOR UPDATE` is rejected.
- The configured database user should still be read-only; SQL filtering is a second layer, not the only one.
- `explain_query` stays read-only. PostgreSQL and SQLite are supported. Oracle returns an explicit error instead of using writable plan tables.
- Integration tests are available behind the `integration` build tag and use environment variables instead of embedding credentials.
