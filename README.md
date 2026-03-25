# db-mcp

Minimal MCP server for databases, implemented in Go. The first driver is Oracle, with the service layer kept generic so PostgreSQL and SQLite can be added without changing the MCP tool surface.

## Features

- MCP tools: `list_connections`, `list_schemas`, `list_tables`, `describe_table`, `execute_query`
- Oracle support via a pure Go driver, so it can connect directly from a Go binary without Oracle Instant Client
- Supports `stdio`, `http`, and `both` transport modes
- Read-only guardrails for ad hoc SQL
- Row, cell, column, and total output truncation to avoid blowing up MCP context

## Config

Use a JSON config file:

```bash
go run ./cmd/dbmcp -config ./dbmcp.example.json
```

Or use environment variables for a single Oracle connection:

```bash
export DBMCP_TRANSPORT=stdio
export DBMCP_ORACLE_JDBC_URL='jdbc:oracle:thin:@unraid.cloud1ful.com:1521:XE'
export DBMCP_ORACLE_USERNAME=readuser
export DBMCP_ORACLE_PASSWORD=readuser
go run ./cmd/dbmcp
```

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
  db-mcp:latest
```

The container defaults to:

- `-transport http`
- `-http-addr :8080`
- MCP endpoint: `http://<host>:8080/mcp`
- Health endpoint: `http://<host>:8080/healthz`

On startup the server now logs:

- enabled transport
- HTTP host, port, and MCP path when HTTP is enabled
- each configured database connection and whether it connected successfully

Override startup flags if needed:

```bash
docker run --rm -p 8081:8081 \
  -e DBMCP_ORACLE_JDBC_URL='jdbc:oracle:thin:@unraid.cloud1ful.com:1521:XE' \
  -e DBMCP_ORACLE_USERNAME=readuser \
  -e DBMCP_ORACLE_PASSWORD=readuser \
  db-mcp:latest -transport http -http-addr :8081
```

## Notes

- `execute_query` only accepts `SELECT` and `WITH` statements.
- Any attempt to send multiple statements or `FOR UPDATE` is rejected.
- The configured database user should still be read-only; SQL filtering is a second layer, not the only one.
- Integration tests are available behind the `integration` build tag and use environment variables instead of embedding credentials.
