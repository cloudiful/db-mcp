# db-mcp

[English](README.md) · [简体中文](README.zh-CN.md)

Rust MCP 数据库服务器。基于官方 Rust MCP SDK（`rmcp`，来自 modelcontextprotocol/rust-sdk 项目）并内嵌 `axum` HTTP 服务器；PostgreSQL 与 SQLite 使用 `sqlx`，Oracle 使用 `oracle-rs`。

## 运行

```bash
cargo run -- -c ./dbmcp.example.json
```

或使用环境变量：

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

不带子命令即启动 MCP 服务器；`dbmcp serve` 是显式写法。

## MCP 工具

默认 `canonical` profile 暴露 4 个工具。任何访问数据库的工具都必须显式传入 `connection` 名称，没有隐式默认连接。先调用 `db_connections`（或读取 `db://connections`），再复用返回的连接名。

### `db_connections`

| 字段 | 含义 |
| --- | --- |
| `action`（必填） | `list` 或 `diagnose` |
| `connection` | 可选的连接名过滤 |
| `refresh` | 仅 `list`：重新探测健康状态而不使用缓存快照 |
| `deep` | 仅 `diagnose`：执行分阶段连接诊断 |

返回 `{"kind":"list"|"diagnose","data":{...}}`。

### `db_introspect`

| 字段 | 含义 |
| --- | --- |
| `action`（必填） | `schemas`、`tables`、`columns`、`constraints`、`indexes`、`comments` 或 `ddl` |
| `connection`（必填） | 连接名 |
| `schema` | 限定查询的 schema/命名空间 |
| `table`（`columns`、`comments`、`ddl` 必填） | 表或视图名 |
| `filter` | `schemas` 与 `tables` 的名称过滤 |
| `limit` | `schemas`、`tables`、`constraints`、`indexes` 的行数上限 |

返回 `{"kind":"<action>","data":{...}}`。DDL、列元数据、约束/索引清单和注释均按驱动规范化；不要用原始 SQL 重建这些信息，它无法跨 PostgreSQL、SQLite 和 Oracle 移植。

### `db_query`

`sql` 与 `queries` 必须二选一：

| 字段 | 含义 |
| --- | --- |
| `connection`（必填） | 连接名 |
| `sql` | 单条语句查询（允许多行与结尾 `;`，不支持 SQL 脚本） |
| `queries` | 1–8 条，每项 `{"sql":"..."}`，按顺序执行；首个失败即停止 |
| `max_rows` | 可选行数上限，受 `limits.hardMaxRows` 约束 |

`db_query` 要求只读连接（见[安全边界](#安全边界)）。返回 `{"kind":"single"|"batch","data":{...}}`。批量失败仍为 `kind: "batch"` 且 `status: "failed"`；其 `error` 保留失败语句的 `index` 与 `message`，并带上共享的稳定字段（`code`、`kind`，以及可选的 `operation`/`timeout`/`cancellation`/`resultUnknown`）。后续语句不再执行。

### `db_explain`

| 字段 | 含义 |
| --- | --- |
| `connection`（必填） | 连接名 |
| `sql`（必填） | 单条语句查询 |

返回该语句的执行计划。原始 SQL 不能替代 `db_explain`、`db_introspect` 或 `db_connections`：查询结果会被截断，而执行计划、DDL 和诊断信息包含各驱动的完整细节。

### 保留的 resources 与 prompts

所有 profile 都保留：

- Resource `db://connections` — 已配置连接及其健康状态
- Resource template `db://table/{connection}/{schema}/{table}` — 合并的列、约束、索引与注释
- Prompt `explore_database` — 安全的分步探索流程
- Prompt `inspect_table` — 检查单表并起草安全只读查询

## 工具 profile 与迁移

在配置中用 `server.toolProfile` 设置，或用 `DBMCP_TOOL_PROFILE` 覆盖：

| Profile | 工具数 | 用途 |
| --- | --- | --- |
| `canonical`（默认） | 4 | 新客户端 |
| `legacy` | 13 | 仍在调用旧名字的既有客户端 |
| `both` | 17 | 迁移期，canonical 与 legacy 并存 |

Profile 在启动时固定；不支持动态 `tools/list_changed`。未知取值会导致启动失败。

legacy 到 canonical 的对应关系：

| Legacy 工具 | Canonical 等价写法 |
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
| `execute_query` | `db_query` 传 `sql` |
| `execute_queries` | `db_query` 传 `queries` |
| `explain_query` | `db_explain` |
| `search_columns` | 无 canonical 等价项；仅在 `legacy`/`both` 下可用 |

## CLI

CLI 直接复用数据库 domain，不经过 MCP transport。命令逐级披露——用 `dbmcp <命令> --help` 查看下一级：

```text
dbmcp [OPTIONS] [COMMAND]
  serve
  connections list [--refresh] [--deep]
  introspect <schemas|tables|columns|constraints|indexes|comments|ddl> \
      --connection NAME [--schema S] [--table T] [--filter F] [--limit N]
  query   --connection NAME (--sql SQL | --file PATH | --stdin) [--max-rows N]
  explain --connection NAME (--sql SQL | --file PATH | --stdin)
```

示例：

```bash
dbmcp -c ./dbmcp.example.json connections list
dbmcp -c ./dbmcp.example.json introspect tables --connection postgres-dev --filter order
dbmcp -c ./dbmcp.example.json query --connection postgres-dev --sql 'select 1'
dbmcp -c ./dbmcp.example.json explain --connection postgres-dev --file query.sql
```

`query` 执行单条只读语句，支持 `--sql`、`--file` 或 `--stdin`（三选一）。批量查询仍由 MCP `db_query` 承担。

## 输出、流与退出码

`--format json`（默认）输出稳定的 JSON 文档；`--format text` 将同一对象渲染为缩进的人类可读树。成功结果写到 stdout，诊断日志与错误负载写到 stderr。

成功：

```json
{"kind":"single","data":{"connection":"postgres-dev","result":{"rows":[]}}}
```

错误是 stderr 上的单行 JSON。`error`、`kind`、`command` 字段保持稳定；`code` 是共享的机器可读 code，超时相关字段仅在错误链携带时出现：

```json
{"error":"...","kind":"timeout","code":"operation_timeout","command":"query","operation":"execute","timeout":true,"cancellation":"unconfirmed","resultUnknown":true}
```

退出码保持不变：

| 码 | 含义 |
| --- | --- |
| `0` | 成功 |
| `1` | 其他内部错误 |
| `2` | 参数错误 |
| `3` | 连接不存在 |
| `4` | 超时 |
| `5` | 后端/数据库错误 |

## 错误契约

MCP 工具失败是工具级结果（`isError: true`），其 `structuredContent`（与可见文本一致）为下方错误文档。参数格式错误和未知工具仍是 JSON-RPC `-32602` invalid-params 错误，绝不会变成工具结果。

| 字段 | 含义 |
| --- | --- |
| `code` | 稳定的机器可读 code：`invalid_params`、`missing_connection`、`operation_timeout`、`backend_error` 或 `internal_error` |
| `kind` | 与 CLI 退出码对应的粗粒度分类：`invalid_args`、`missing_connection`、`timeout`、`backend` 或 `internal` |
| `message` | 脱敏且限制长度的调用方面向消息 |
| `operation` | 可选的超时逻辑阶段（`acquire`、`connect`、`begin`、`describe`、`execute`、`fetch`、`commit`、`rollback`、`query`、`explain`、`metadata`） |
| `connection` | 可选的调用连接名 |
| `timeout` | 仅超时时存在且为 `true` |
| `cancellation` | 可选，`confirmed` 或 `unconfirmed` |
| `resultUnknown` | 可选；写操作超时后其结果可能未知时为 `true` |

`limits.queryTimeout`（默认 `60s`）是单个逻辑操作的统一截止时间：acquire、begin、describe、execute/fetch、commit/rollback 共享剩余时间，而不是每阶段重新计时；批量查询的所有语句共享同一截止时间。超时时：

- `cancellation: "confirmed"` 表示引擎或驱动已报告语句停止（PostgreSQL `statement_timeout` 的 SQLSTATE `57014`、SQLite progress handler 中断）。
- `cancellation: "unconfirmed"` 表示客户端截止时间先到；物理连接被丢弃而不放回连接池，语句可能仍在执行。当前 Oracle 驱动无法验证中断，始终报告 `unconfirmed`。
- `resultUnknown: true` 表示写操作可能已生效也可能未生效，不要盲目重试。

完整 cause chain、原始 SQL 与 DSN 凭据不会进入 MCP 或 CLI 响应；脱敏后的完整链只写入服务端日志。诊断（`db_connections` 的 `diagnose`、`connections list --deep`）仍返回脱敏后的探测链，以保证可诊断性。

## 安全边界

- 4 个 canonical 工具都标注为只读。`db_query` 与 CLI `query` 还会在**服务端**通过 `supports_read_only_batch` 策略强制只读；MCP annotation 只是提示，不构成安全控制。
- PostgreSQL 与 Oracle 要求 `readOnly: true`。SQLite 要求 `readOnly: true` **且**使用只读 DSN（`mode=ro` 或 `immutable=1`）；`mode=rwc` 会被拒绝。
- legacy `execute_query`/`execute_queries` 是唯一可能写入的入口，且仅当连接配置为 `readOnly: false` 时生效。`db_explain` 属于只读操作，不受批量只读门禁限制。
- CLI 错误输出会脱敏凭据。

## 跨库注意事项

- PostgreSQL 与 Oracle 连接默认 `readOnly: true`，查询在只读事务中执行。
- SQLite 此处没有事务级只读强制；单条写入由 DSN 与数据库权限控制。`readOnly: true` 但 DSN 可写时会在启动时打印警告。
- 没有隐式默认数据库选择；始终显式传入连接名。
- 查询元数据请使用 `db://table/{connection}/{schema}/{table}` 或 `db_introspect`，不要直接查各引擎不同的系统目录表。

## 端点

启用 HTTP 时：`/mcp`、`/healthz`、`/readyz`。

## 发布

- 版本 tag 会向 GitHub Releases 发布 `x86_64-unknown-linux-gnu` 与 `aarch64-unknown-linux-gnu` 二进制归档
- 容器镜像发布到 `ghcr.io/<owner>/<repository>`，支持 `linux/amd64` 与 `linux/arm64`

## 测试

```bash
cargo fmt --all --check
SQLX_OFFLINE=true cargo check --locked --all-targets
SQLX_OFFLINE=true cargo clippy --all-targets --locked -- -D warnings
SQLX_OFFLINE=true cargo test --locked --all-targets
```

除非设置 `DBMCP_TEST_POSTGRES_DSN`，或 `DBMCP_TEST_ORACLE_JDBC_URL`、`DBMCP_TEST_ORACLE_USERNAME`、`DBMCP_TEST_ORACLE_PASSWORD`，否则跳过 PostgreSQL 与 Oracle 集成测试。

## 说明

- Transport 模式：`stdio`、`http` 或 `both`（通过配置或 `DBMCP_TRANSPORT`）
- HTTP 在 `/mcp` 提供 MCP Streamable HTTP 端点，另有 `/healthz` 与 `/readyz`；当 HTTP 绑定非 loopback 地址时默认要求 bearer 认证（`DBMCP_HTTP_AUTH_MODE=bearer` + `DBMCP_HTTP_AUTH_TOKEN`）。未知的 `DBMCP_HTTP_AUTH_MODE` 取值会导致启动错误
- CORS 默认关闭；通过 `httpCorsAllowedOrigins` 或逗号分隔的 `DBMCP_HTTP_CORS_ALLOWED_ORIGINS` 按 origin 开启
- HTTP 服务期间健康状态每 30s 后台刷新；`db_connections` 的 `list`/`/readyz` 返回上次缓存探测结果，除非传入 `refresh: true`，`diagnose` 则执行按需深度检查
