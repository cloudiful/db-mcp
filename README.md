# db-mcp

[English](README.md) · [简体中文](README.zh-CN.md)

Rust MCP database server. Built on the official Rust MCP SDK (`rmcp`, from the modelcontextprotocol/rust-sdk project) with an embedded `axum` HTTP server; uses `sqlx` for PostgreSQL and SQLite and `oracle-rs` for Oracle.

## Run

```bash
cargo run -- -c ./dbmcp.example.json
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

Running with no subcommand starts the MCP server; `dbmcp serve` is the explicit form.

## MCP tools

The default `canonical` profile exposes four tools. Every database tool that touches a database needs an explicit `connection` name; there is no implicit default connection. Call `db_connections` (or read `db://connections`) first and reuse one of the returned names.

### `db_connections`

| Field | Meaning |
| --- | --- |
| `action` (required) | `list` or `diagnose` |
| `connection` | Optional connection name filter |
| `refresh` | `list` only: re-probe health instead of using the cached snapshot |
| `deep` | `diagnose` only: run staged connection diagnostics |

Returns `{"kind":"list"|"diagnose","data":{...}}`.

### `db_introspect`

| Field | Meaning |
| --- | --- |
| `action` (required) | `schemas`, `tables`, `columns`, `constraints`, `indexes`, `comments`, or `ddl` |
| `connection` (required) | Connection name |
| `schema` | Schema/namespace to scope the lookup |
| `table` (required for `columns`, `comments`, `ddl`) | Table or view name |
| `filter` | Name filter for `schemas` and `tables` |
| `limit` | Row cap for `schemas`, `tables`, `constraints`, and `indexes` |

Returns `{"kind":"<action>","data":{...}}`. DDL, column metadata, constraint/index listings, and comments are normalized per driver; do not reconstruct them with raw SQL, which is not portable across PostgreSQL, SQLite, and Oracle.

### `db_query`

Exactly one of `sql` or `queries` must be present:

| Field | Meaning |
| --- | --- |
| `connection` (required) | Connection name |
| `sql` | One single-statement query (multiline and a trailing `;` are allowed; SQL scripts are not) |
| `queries` | 1–8 items, each `{"sql":"..."}`, run sequentially; execution stops at the first failure |
| `max_rows` | Optional row cap, bounded by `limits.hardMaxRows` |

`db_query` requires a read-only connection (see [Safety](#safety)). Returns `{"kind":"single"|"batch","data":{...}}`.

### `db_explain`

| Field | Meaning |
| --- | --- |
| `connection` (required) | Connection name |
| `sql` (required) | One single-statement query |

Returns the execution plan for the statement. Raw SQL cannot replace `db_explain`, `db_introspect`, or `db_connections`: query results are truncated, while explain plans, DDL, and diagnostics carry the driver-specific detail.

### Resources and prompts

Resources and prompts are retained in every profile:

- Resource `db://connections` — configured connections and their health
- Resource template `db://table/{connection}/{schema}/{table}` — combined columns, constraints, indexes, and comments
- Prompt `explore_database` — safe step-by-step exploration workflow
- Prompt `inspect_table` — inspect one table and draft a safe read-only query

## Tool profiles and migration

Set the profile in config as `server.toolProfile` or override with `DBMCP_TOOL_PROFILE`:

| Profile | Tools | Use |
| --- | --- | --- |
| `canonical` (default) | 4 | New clients |
| `legacy` | 13 | Existing clients that still call the original names |
| `both` | 17 | Migration window, canonical and legacy together |

The profile is fixed at startup; there is no dynamic `tools/list_changed`. Unknown values fail startup.

Legacy-to-canonical mapping:

| Legacy tool | Canonical equivalent |
| --- | --- |
| `list_connections` | `db_connections` `action: "list"` |
| `diagnose_connection` | `db_connections` `action: "diagnose"` |
| `list_schemas` | `db_introspect` `action: "schemas"` |
| `list_tables` | `db_introspect` `action: "tables"` |
| `describe_table` | `db_introspect` `action: "columns"` |
| `list_constraints` | `db_introspect` `action: "constraints"` |
| `list_indexes` | `db_introspect` `action: "indexes"` |
| `get_table_comments` | `db_introspect` `action: "comments"` |
| `get_table_ddl` | `db_introspect` `action: "ddl"` |
| `execute_query` | `db_query` with `sql` |
| `execute_queries` | `db_query` with `queries` |
| `explain_query` | `db_explain` |
| `search_columns` | No canonical equivalent; available only under `legacy`/`both` |

## CLI

The CLI shares the database domain directly and never goes through MCP transport. Commands disclose progressively — run `dbmcp <command> --help` for the next level:

```text
dbmcp [OPTIONS] [COMMAND]
  serve
  connections list [--refresh] [--deep]
  introspect <schemas|tables|columns|constraints|indexes|comments|ddl> \
      --connection NAME [--schema S] [--table T] [--filter F] [--limit N]
  query   --connection NAME (--sql SQL | --file PATH | --stdin) [--max-rows N]
  explain --connection NAME (--sql SQL | --file PATH | --stdin)
```

Examples:

```bash
dbmcp -c ./dbmcp.example.json connections list
dbmcp -c ./dbmcp.example.json introspect tables --connection postgres-dev --filter order
dbmcp -c ./dbmcp.example.json query --connection postgres-dev --sql 'select 1'
dbmcp -c ./dbmcp.example.json explain --connection postgres-dev --file query.sql
```

`query` runs one read-only statement and supports `--sql`, `--file`, or `--stdin` (exactly one). Batches stay on MCP `db_query`.

## Output, streams, and exit codes

`--format json` (default) writes a stable JSON document; `--format text` renders the same object as an indented human-readable tree. Successful results go to stdout, diagnostics and the error payload go to stderr.

Success:

```json
{"kind":"single","data":{"connection":"postgres-dev","result":{"rows":[]}}}
```

Errors are a single JSON line on stderr:

```json
{"error":"...","kind":"invalid_args","command":"query"}
```

Exit codes:

| Code | Meaning |
| --- | --- |
| `0` | Success |
| `1` | Other internal error |
| `2` | Invalid arguments |
| `3` | Connection not found |
| `4` | Timeout |
| `5` | Backend/database error |

## Safety

- The canonical four tools are annotated read-only. `db_query` and the CLI `query` command additionally enforce read-only **server-side** by checking a `supports_read_only_batch` policy; the MCP annotation is a hint, not the control.
- PostgreSQL and Oracle require `readOnly: true`. SQLite requires `readOnly: true` **and** a read-only DSN (`mode=ro` or `immutable=1`); `mode=rwc` is refused.
- Legacy `execute_query`/`execute_queries` remain the only entries that can write, and only when a connection is configured with `readOnly: false`. `db_explain` is a read-only operation and is available without the batch read-only gate.
- Credentials are redacted from CLI error output.

## Database notes

- PostgreSQL and Oracle connections default to `readOnly: true` and run queries inside read-only transactions.
- SQLite has no transaction-level read-only enforcement here; single-statement writes are governed by the DSN and database permissions. A `readOnly: true` connection with a writable DSN logs a startup warning.
- No implicit default database selection; always pass an explicit connection name.
- Use `db://table/{connection}/{schema}/{table}` or `db_introspect` for metadata rather than querying catalog tables, which differ per engine.

## Endpoints

When HTTP is enabled: `/mcp`, `/healthz`, `/readyz`.

## Releases

- Version tags publish `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` binary archives to GitHub Releases
- The container image is published to `ghcr.io/<owner>/<repository>` with `linux/amd64` and `linux/arm64` support

## Tests

```bash
cargo fmt --all --check
SQLX_OFFLINE=true cargo check --locked --all-targets
SQLX_OFFLINE=true cargo clippy --all-targets --locked -- -D warnings
SQLX_OFFLINE=true cargo test --locked --all-targets
```

PostgreSQL and Oracle integration tests are skipped unless `DBMCP_TEST_POSTGRES_DSN` or `DBMCP_TEST_ORACLE_JDBC_URL`, `DBMCP_TEST_ORACLE_USERNAME`, and `DBMCP_TEST_ORACLE_PASSWORD` are set.

## Notes

- Transport modes: `stdio`, `http`, or `both` (via config or `DBMCP_TRANSPORT`)
- HTTP serves the MCP Streamable HTTP endpoint at `/mcp`, plus `/healthz` and `/readyz`; bearer auth is required by default whenever HTTP binds a non-loopback address (`DBMCP_HTTP_AUTH_MODE=bearer` + `DBMCP_HTTP_AUTH_TOKEN`). An unknown `DBMCP_HTTP_AUTH_MODE` value is a startup error
- CORS is off by default; opt in per origin with `httpCorsAllowedOrigins` or the comma-separated `DBMCP_HTTP_CORS_ALLOWED_ORIGINS` env var
- Health state is refreshed in the background (every 30s) while HTTP is serving; `db_connections` `list`/`/readyz` return the last cached probe unless `refresh: true` is passed, and `diagnose` runs an on-demand deep check
