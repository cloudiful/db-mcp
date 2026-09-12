//! CLI integration tests: run the real `dbmcp` binary as a child process
//! against a temporary read-only SQLite fixture. Environment is passed per
//! child process, so no in-process `set_var` (and no cross-test pollution).

use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A deterministic CPU-bound read-only query that outlives a short test budget.
const SLOW_SQL: &str = "with recursive counter(value) as (\
    select 1 union all select value + 1 from counter where value < 5000000\
) select count(*) as value from counter";

fn temp_dir(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "dbmcp-cli-{label}-{}-{sequence}-{suffix}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_config(dir: &Path, name: &str, dsn: &str, read_only: bool) -> PathBuf {
    let config = json!({
        "connections": [
            {
                "name": "sqlite",
                "driver": "sqlite",
                "sqlite": { "dsn": dsn, "readOnly": read_only },
            }
        ]
    });
    let path = dir.join(name);
    fs::write(
        &path,
        serde_json::to_string_pretty(&config).expect("serialize config"),
    )
    .expect("write config");
    path
}

/// A read-only config with an explicit `limits.queryTimeout`, for exercising
/// the timeout error contract end to end.
fn write_timeout_config(dir: &Path, name: &str, dsn: &str, query_timeout: &str) -> PathBuf {
    let config = json!({
        "limits": { "queryTimeout": query_timeout },
        "connections": [
            {
                "name": "sqlite",
                "driver": "sqlite",
                "sqlite": { "dsn": dsn, "readOnly": true },
            }
        ]
    });
    let path = dir.join(name);
    fs::write(
        &path,
        serde_json::to_string_pretty(&config).expect("serialize config"),
    )
    .expect("write config");
    path
}

/// A temp SQLite database plus read-only and writable config files. The
/// directory is removed when the fixture drops.
struct Fixture {
    dir: PathBuf,
    ro_config: PathBuf,
    rw_config: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let dir = temp_dir(label);
        let db_path = dir.join("fixture.db");
        let writable = format!("sqlite://{}?mode=rwc", db_path.display());
        let read_only = format!("sqlite://{}?mode=ro", db_path.display());

        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        runtime.block_on(async {
            let pool = sqlx::SqlitePool::connect(&writable)
                .await
                .expect("create fixture database");
            sqlx::query("create table probe (value integer)")
                .execute(&pool)
                .await
                .expect("create probe table");
            sqlx::query("insert into probe (value) values (42)")
                .execute(&pool)
                .await
                .expect("seed probe table");
            pool.close().await;
        });

        let ro_config = write_config(&dir, "ro.json", &read_only, true);
        let rw_config = write_config(&dir, "rw.json", &writable, true);
        Self {
            dir,
            ro_config,
            rw_config,
        }
    }

    fn ro_config(&self) -> &Path {
        &self.ro_config
    }

    fn rw_config(&self) -> &Path {
        &self.rw_config
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn base_command(config: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_dbmcp"));
    command.arg("--config").arg(config);
    for key in [
        "DBMCP_CONFIG",
        "DBMCP_TRANSPORT",
        "DBMCP_HTTP_ADDR",
        "DBMCP_TOOL_PROFILE",
        "DBMCP_SQLITE_DSN",
        "DBMCP_SQLITE_NAME",
        "DBMCP_POSTGRES_DSN",
        "DBMCP_ORACLE_JDBC_URL",
    ] {
        command.env_remove(key);
    }
    command
}

fn dbmcp(config: &Path, args: &[&str]) -> Output {
    base_command(config).args(args).output().expect("run dbmcp")
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr_text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn stdout_json(output: &Output) -> Value {
    let text = stdout_text(output);
    serde_json::from_str(&text).unwrap_or_else(|err| {
        panic!(
            "stdout is not a single JSON document: {err}\nstdout: {text}\nstderr: {}",
            stderr_text(output)
        )
    })
}

/// The structured error payload shares stderr with tracing logs, so scan from
/// the end for the last JSON-object line rather than parsing the whole stream.
fn stderr_json(output: &Output) -> Value {
    let text = stderr_text(output);
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.starts_with('{')
            && let Ok(value) = serde_json::from_str::<Value>(trimmed)
        {
            return value;
        }
    }
    panic!("stderr has no error JSON object:\n{text}");
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "unexpected exit code\nstdout: {}\nstderr: {}",
        stdout_text(output),
        stderr_text(output)
    );
}

#[test]
fn help_lists_progressive_subcommands() {
    let fixture = Fixture::new("help");

    let output = dbmcp(fixture.ro_config(), &["--help"]);
    assert_exit(&output, 0);
    let text = stdout_text(&output);
    for expected in ["serve", "connections", "introspect", "query", "explain"] {
        assert!(text.contains(expected), "help missing {expected}:\n{text}");
    }

    let output = dbmcp(fixture.ro_config(), &["connections", "list", "--help"]);
    assert_exit(&output, 0);
    let text = stdout_text(&output);
    assert!(text.contains("--refresh"), "missing --refresh:\n{text}");
    assert!(text.contains("--deep"), "missing --deep:\n{text}");

    let output = dbmcp(fixture.ro_config(), &["introspect", "tables", "--help"]);
    assert_exit(&output, 0);
    let text = stdout_text(&output);
    for expected in ["--connection", "--schema", "--table", "--filter", "--limit"] {
        assert!(
            text.contains(expected),
            "introspect help missing {expected}:\n{text}"
        );
    }
}

#[test]
fn connections_list_emits_connection_json() {
    let fixture = Fixture::new("connections");

    let output = dbmcp(fixture.ro_config(), &["connections", "list"]);
    assert_exit(&output, 0);
    let value = stdout_json(&output);
    assert_eq!(value["kind"], "list");
    let connections = value["data"]["connections"]
        .as_array()
        .expect("connections array");
    assert!(
        connections.iter().any(|conn| conn["name"] == "sqlite"),
        "expected sqlite connection: {value}"
    );

    let output = dbmcp(fixture.ro_config(), &["connections", "list", "--deep"]);
    assert_exit(&output, 0);
    let value = stdout_json(&output);
    assert_eq!(value["kind"], "diagnose");
    assert_eq!(value["data"]["diagnostics"].as_array().unwrap().len(), 1);
}

#[test]
fn introspect_actions_emit_typed_json() {
    let fixture = Fixture::new("introspect");
    for (action, kind) in [
        ("schemas", "schemas"),
        ("tables", "tables"),
        ("columns", "columns"),
        ("constraints", "constraints"),
        ("indexes", "indexes"),
        ("comments", "comments"),
        ("ddl", "ddl"),
    ] {
        let output = dbmcp(
            fixture.ro_config(),
            &[
                "introspect",
                action,
                "--connection",
                "sqlite",
                "--table",
                "probe",
            ],
        );
        assert_exit(&output, 0);
        let value = stdout_json(&output);
        assert_eq!(value["kind"], kind, "introspect {action}");
        assert_eq!(value["data"]["connection"], "sqlite", "introspect {action}");
    }
}

#[test]
fn query_supports_sql_file_and_stdin_sources() {
    let fixture = Fixture::new("query");

    let output = dbmcp(
        fixture.ro_config(),
        &[
            "query",
            "--connection",
            "sqlite",
            "--sql",
            "select 1 as value",
        ],
    );
    assert_exit(&output, 0);
    let value = stdout_json(&output);
    assert_eq!(value["kind"], "single");
    assert_eq!(value["data"]["result"]["rows"][0]["value"], json!(1));

    let file = fixture.path("query.sql");
    fs::write(&file, "select 7 as value;").expect("write sql file");
    let output = dbmcp(
        fixture.ro_config(),
        &[
            "query",
            "--connection",
            "sqlite",
            "--file",
            file.to_str().expect("utf-8 path"),
        ],
    );
    assert_exit(&output, 0);
    let value = stdout_json(&output);
    assert_eq!(value["data"]["result"]["rows"][0]["value"], json!(7));

    let mut command = base_command(fixture.ro_config());
    command
        .args(["query", "--connection", "sqlite", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn query from stdin");
    child
        .stdin
        .take()
        .expect("child stdin")
        .write_all(b"select 9 as value")
        .expect("write sql to stdin");
    let output = child.wait_with_output().expect("wait query from stdin");
    assert_exit(&output, 0);
    let value = stdout_json(&output);
    assert_eq!(value["data"]["result"]["rows"][0]["value"], json!(9));
}

#[test]
fn explain_emits_execution_plan() {
    let fixture = Fixture::new("explain");
    let output = dbmcp(
        fixture.ro_config(),
        &[
            "explain",
            "--connection",
            "sqlite",
            "--sql",
            "select * from probe",
        ],
    );
    assert_exit(&output, 0);
    let value = stdout_json(&output);
    assert_eq!(value["connection"], "sqlite");
    assert!(
        value["explain"]["planLines"]
            .as_array()
            .is_some_and(|lines| !lines.is_empty()),
        "expected plan lines: {value}"
    );
}

#[test]
fn text_format_renders_human_projection() {
    let fixture = Fixture::new("text");
    let output = dbmcp(
        fixture.ro_config(),
        &[
            "--format",
            "text",
            "introspect",
            "tables",
            "--connection",
            "sqlite",
        ],
    );
    assert_exit(&output, 0);
    let text = stdout_text(&output);
    assert!(text.contains("kind: tables"), "{text}");
    assert!(
        serde_json::from_str::<Value>(text.trim()).is_err(),
        "text output must not be JSON: {text}"
    );
}

#[test]
fn errors_use_documented_exit_codes() {
    let fixture = Fixture::new("errors");

    // Missing connection -> 3 with a structured stderr payload.
    let output = dbmcp(
        fixture.ro_config(),
        &["query", "--connection", "ghost", "--sql", "select 1"],
    );
    assert_exit(&output, 3);
    assert!(
        output.stdout.is_empty(),
        "stdout must stay clean on failure"
    );
    let error = stderr_json(&output);
    assert_eq!(error["kind"], "missing_connection");
    assert_eq!(error["code"], "missing_connection");
    assert_eq!(error["command"], "query");

    // Writable connection is refused by the read-only gate -> 2.
    let output = dbmcp(
        fixture.rw_config(),
        &["query", "--connection", "sqlite", "--sql", "select 1"],
    );
    assert_exit(&output, 2);
    assert!(
        output.stdout.is_empty(),
        "stdout must stay clean on failure"
    );
    let error = stderr_json(&output);
    assert_eq!(error["kind"], "invalid_args");
    assert_eq!(error["code"], "invalid_params");
    assert!(
        error["error"].as_str().unwrap().contains("read-only"),
        "unexpected error: {error}"
    );

    // Missing SQL source -> 2.
    let output = dbmcp(fixture.ro_config(), &["query", "--connection", "sqlite"]);
    assert_exit(&output, 2);
    assert!(
        output.stdout.is_empty(),
        "stdout must stay clean on failure"
    );
    assert_eq!(stderr_json(&output)["kind"], "invalid_args");

    // Table-scoped introspect without a table -> 2.
    let output = dbmcp(
        fixture.ro_config(),
        &["introspect", "columns", "--connection", "sqlite"],
    );
    assert_exit(&output, 2);
    assert!(
        output.stdout.is_empty(),
        "stdout must stay clean on failure"
    );
    assert_eq!(stderr_json(&output)["kind"], "invalid_args");

    // A clap-level invalid parameter also exits 2.
    let output = dbmcp(
        fixture.ro_config(),
        &["--format", "yaml", "connections", "list"],
    );
    assert_exit(&output, 2);
}

#[test]
fn backend_errors_are_structured() {
    let fixture = Fixture::new("backend");
    let output = dbmcp(
        fixture.ro_config(),
        &[
            "query",
            "--connection",
            "sqlite",
            "--sql",
            "select * from missing_cli_table",
        ],
    );
    assert_exit(&output, 5);
    assert!(
        output.stdout.is_empty(),
        "stdout must stay clean on failure"
    );
    let error = stderr_json(&output);
    assert_eq!(error["kind"], "backend");
    assert_eq!(error["code"], "backend_error");
    assert_eq!(error["command"], "query");
    assert!(
        error["error"]
            .as_str()
            .is_some_and(|message| message.contains("missing_cli_table")),
        "unexpected error: {error}"
    );
    assert!(error.get("timeout").is_none());
}

#[test]
fn timeout_errors_are_structured() {
    let fixture = Fixture::new("timeout");
    let read_only = format!(
        "sqlite://{}?mode=ro",
        fixture.dir.join("fixture.db").display()
    );
    let config = write_timeout_config(&fixture.dir, "timeout.json", &read_only, "300ms");

    let output = dbmcp(
        &config,
        &["query", "--connection", "sqlite", "--sql", SLOW_SQL],
    );
    assert_exit(&output, 4);
    assert!(
        output.stdout.is_empty(),
        "stdout must stay clean on failure"
    );
    let error = stderr_json(&output);
    assert_eq!(error["kind"], "timeout");
    assert_eq!(error["code"], "operation_timeout");
    assert_eq!(error["timeout"], true);
    assert_eq!(error["command"], "query");
    assert!(
        error["cancellation"].is_string(),
        "timeout must carry a typed cancellation state: {error}"
    );
    assert!(
        error["error"]
            .as_str()
            .is_some_and(|message| message.contains("timeout")),
        "unexpected error: {error}"
    );
}
