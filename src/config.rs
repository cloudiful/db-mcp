use anyhow::{Context, Result, bail};
use axum::http::HeaderValue;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, fs, path::PathBuf, time::Duration};

use crate::cli::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum TransportMode {
    Stdio,
    Http,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum HttpAuthMode {
    #[default]
    None,
    Bearer,
}

/// Which MCP tool surface the server exposes. `Canonical` is the default
/// four-tool surface; `Legacy` keeps the original thirteen tools; `Both`
/// exposes the union for migration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolProfile {
    #[default]
    Canonical,
    Legacy,
    Both,
}

impl ToolProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Legacy => "legacy",
            Self::Both => "both",
        }
    }
}

/// Output encoding for the progressive CLI. This is a command-line option and
/// deliberately does not participate in the server JSON config.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Json,
    Text,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Result<Self> {
        match value.to_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "text" => Ok(Self::Text),
            other => bail!("invalid output format {other:?}; expected \"json\" or \"text\""),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Text => "text",
        }
    }
}

/// Shared statement cap for the canonical batch query path. MCP canonical
/// `db_query` and any future CLI batch handling must use this single value.
pub const MAX_BATCH_QUERIES: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HttpAuthConfig {
    pub mode: HttpAuthMode,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerConfig {
    pub name: String,
    pub version: String,
    pub debug: bool,
    pub transport: TransportMode,
    pub http_addr: String,
    pub http_path: String,
    pub http_auth: HttpAuthConfig,
    pub http_cors_allowed_origins: Vec<String>,
    pub tool_profile: ToolProfile,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            name: "db-mcp".to_string(),
            version: "0.1.0".to_string(),
            debug: false,
            transport: TransportMode::Stdio,
            http_addr: "0.0.0.0:8080".to_string(),
            http_path: "/mcp".to_string(),
            http_auth: HttpAuthConfig::default(),
            http_cors_allowed_origins: vec![],
            tool_profile: ToolProfile::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LimitsConfig {
    pub default_max_rows: usize,
    pub hard_max_rows: usize,
    pub max_columns: usize,
    pub max_cell_chars: usize,
    pub max_total_chars: usize,
    pub explain_max_lines: usize,
    pub explain_max_total_chars: usize,
    pub query_timeout: String,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            default_max_rows: 5,
            hard_max_rows: 20,
            max_columns: 40,
            max_cell_chars: 512,
            max_total_chars: 1000,
            explain_max_lines: 12,
            explain_max_total_chars: 4000,
            query_timeout: "60s".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct Config {
    pub server: ServerConfig,
    pub limits: LimitsConfig,
    pub connections: Vec<ConnectionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[derive(Default)]
pub struct ConnectionConfig {
    pub name: String,
    pub driver: String,
    pub oracle: Option<OracleConfig>,
    pub postgres: Option<PostgresConfig>,
    pub sqlite: Option<SQLiteConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OracleConfig {
    pub jdbc_url: String,
    pub username: String,
    pub password: String,
    pub read_only: bool,
}

impl Default for OracleConfig {
    fn default() -> Self {
        Self {
            jdbc_url: String::new(),
            username: String::new(),
            password: String::new(),
            read_only: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PostgresConfig {
    pub dsn: String,
    pub schema: String,
    pub read_only: bool,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            dsn: String::new(),
            schema: String::new(),
            read_only: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SQLiteConfig {
    pub dsn: String,
    pub read_only: bool,
}

impl Default for SQLiteConfig {
    fn default() -> Self {
        Self {
            dsn: String::new(),
            read_only: true,
        }
    }
}

#[derive(Debug, Parser)]
pub struct Cli {
    #[arg(short = 'c', long, env = "DBMCP_CONFIG", global = true)]
    pub config: Option<PathBuf>,
    #[arg(long, global = true)]
    pub transport: Option<TransportMode>,
    #[arg(long = "http-addr", global = true)]
    pub http_addr: Option<String>,
    #[arg(long = "http-path", global = true)]
    pub http_path: Option<String>,
    #[arg(long, global = true)]
    pub debug: Option<bool>,
    /// Result encoding for CLI subcommands; the MCP server is unaffected.
    #[arg(long, value_enum, default_value = "json", global = true)]
    pub format: OutputFormat,
    /// Progressive CLI commands; omitting the subcommand starts the MCP server.
    #[command(subcommand)]
    pub command: Option<Command>,
}

impl Config {
    pub fn load(cli: &Cli) -> Result<Self> {
        let mut cfg = if let Some(path) = &cli.config {
            let data = fs::read_to_string(path)
                .with_context(|| format!("read config {}", path.display()))?;
            serde_json::from_str::<Config>(&data).context("parse config")?
        } else {
            Config::default()
        };

        apply_env(&mut cfg)?;

        if let Some(transport) = cli.transport {
            cfg.server.transport = transport;
        }
        if let Some(http_addr) = &cli.http_addr {
            cfg.server.http_addr = http_addr.clone();
        }
        if let Some(http_path) = &cli.http_path {
            cfg.server.http_path = http_path.clone();
        }
        if let Some(debug) = cli.debug {
            cfg.server.debug = debug;
        }

        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&mut self) -> Result<()> {
        if self.server.name.trim().is_empty() {
            self.server.name = "db-mcp".to_string();
        }
        if self.server.version.trim().is_empty() {
            self.server.version = "0.1.0".to_string();
        }
        let http_addr = self.server.http_addr.trim();
        if http_addr.is_empty() {
            self.server.http_addr = "0.0.0.0:8080".to_string();
        } else {
            self.server.http_addr = normalize_listen_addr(http_addr);
        }
        if self.server.http_path.trim().is_empty() || !self.server.http_path.starts_with('/') {
            self.server.http_path = "/mcp".to_string();
        }

        if self.limits.default_max_rows == 0 {
            self.limits.default_max_rows = 5;
        }
        if self.limits.hard_max_rows == 0 {
            self.limits.hard_max_rows = 20;
        }
        if self.limits.default_max_rows > self.limits.hard_max_rows {
            self.limits.default_max_rows = self.limits.hard_max_rows;
        }
        if self.limits.max_columns == 0 {
            self.limits.max_columns = 40;
        }
        if self.limits.max_cell_chars == 0 {
            self.limits.max_cell_chars = 512;
        }
        if self.limits.max_total_chars == 0 {
            self.limits.max_total_chars = 1000;
        }
        if self.limits.explain_max_lines == 0 {
            self.limits.explain_max_lines = 12;
        }
        if self.limits.explain_max_total_chars == 0 {
            self.limits.explain_max_total_chars = 4000;
        }
        if self.limits.query_timeout.trim().is_empty() {
            self.limits.query_timeout = "60s".to_string();
        }
        humantime::parse_duration(&self.limits.query_timeout).context("invalid query_timeout")?;

        if self.connections.is_empty() {
            bail!("at least one connection must be configured");
        }

        let mut seen = BTreeMap::new();
        for (idx, conn) in self.connections.iter_mut().enumerate() {
            if conn.name.trim().is_empty() {
                conn.name = default_connection_name(&conn.driver, idx);
            }
            conn.driver = conn.driver.to_lowercase();
            if seen.insert(conn.name.clone(), ()).is_some() {
                bail!("duplicate connection name {}", conn.name);
            }

            match conn.driver.as_str() {
                "oracle" => {
                    let cfg = conn.oracle.as_ref().context("oracle config missing")?;
                    if cfg.jdbc_url.trim().is_empty()
                        || cfg.username.trim().is_empty()
                        || cfg.password.trim().is_empty()
                    {
                        bail!("oracle connection requires jdbc_url, username, and password");
                    }
                }
                "postgres" => {
                    let cfg = conn.postgres.as_ref().context("postgres config missing")?;
                    if cfg.dsn.trim().is_empty() {
                        bail!("postgres connection requires dsn");
                    }
                }
                "sqlite" => {
                    let cfg = conn.sqlite.as_ref().context("sqlite config missing")?;
                    if cfg.dsn.trim().is_empty() {
                        bail!("sqlite connection requires dsn");
                    }
                }
                other => bail!("unsupported driver {}", other),
            }
        }

        if self.server.http_auth.mode == HttpAuthMode::Bearer {
            self.server.http_auth.token = self.server.http_auth.token.trim().to_string();
            if self.server.http_auth.token.is_empty() {
                bail!("http_auth mode bearer requires a non-empty http_auth token");
            }
        }
        if matches!(
            self.server.transport,
            TransportMode::Http | TransportMode::Both
        ) {
            let (host, _port) = split_host_port(&self.server.http_addr);
            if !is_loopback_host(&host) && self.server.http_auth.mode == HttpAuthMode::None {
                bail!(
                    "http transport bound to non-loopback address {} requires http_auth mode \"bearer\"; set DBMCP_HTTP_AUTH_MODE=bearer and DBMCP_HTTP_AUTH_TOKEN",
                    self.server.http_addr
                );
            }
        }
        for origin in &self.server.http_cors_allowed_origins {
            HeaderValue::from_str(origin)
                .with_context(|| format!("invalid http_cors_allowed_origins entry {origin:?}"))?;
        }

        Ok(())
    }

    pub fn query_timeout(&self) -> Result<Duration> {
        Ok(humantime::parse_duration(&self.limits.query_timeout)?)
    }
}

fn default_connection_name(driver: &str, idx: usize) -> String {
    let suffix = idx + 1;
    match driver.to_lowercase().as_str() {
        "oracle" => format!("oracle-{suffix}"),
        "postgres" => format!("postgres-{suffix}"),
        "sqlite" => format!("sqlite-{suffix}"),
        other => format!("{other}-{suffix}"),
    }
}

/// Normalize a listen address into `host:port` form, defaulting an empty host
/// to `0.0.0.0` (e.g. `:8080` becomes `0.0.0.0:8080`).
pub fn normalize_listen_addr(addr: &str) -> String {
    if addr.is_empty() {
        return "0.0.0.0:8080".to_string();
    }
    match addr.rsplit_once(':') {
        Some((host, _port)) if !host.is_empty() => addr.to_string(),
        Some((_, port)) => format!("0.0.0.0:{port}"),
        None => addr.to_string(),
    }
}

/// Split a normalized `host:port` address, stripping IPv6 brackets from the host.
pub(crate) fn split_host_port(addr: &str) -> (String, String) {
    let addr = normalize_listen_addr(addr);
    match addr.rsplit_once(':') {
        Some((host, port)) => (host.trim_matches(['[', ']']).to_string(), port.to_string()),
        None => (addr, String::new()),
    }
}

/// Whether `host` is a loopback address where unauthenticated HTTP is acceptable.
pub(crate) fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "::1"
        || host.starts_with("127.")
        || host.starts_with("::ffff:127.")
}

fn parse_auth_mode(value: &str) -> Result<HttpAuthMode> {
    match value.to_lowercase().as_str() {
        "none" => Ok(HttpAuthMode::None),
        "bearer" => Ok(HttpAuthMode::Bearer),
        other => bail!("invalid http_auth mode {other:?}; expected \"none\" or \"bearer\""),
    }
}

fn apply_env(cfg: &mut Config) -> Result<()> {
    if let Some(parsed) = env::var("DBMCP_DEBUG").ok().and_then(|v| v.parse().ok()) {
        cfg.server.debug = parsed;
    }
    if let Some(parsed) = env::var("DBMCP_TRANSPORT")
        .ok()
        .and_then(|v| parse_transport(&v).ok())
    {
        cfg.server.transport = parsed;
    }
    if let Ok(v) = env::var("DBMCP_HTTP_ADDR") {
        cfg.server.http_addr = v;
    }
    if let Ok(v) = env::var("DBMCP_HTTP_PATH") {
        cfg.server.http_path = v;
    }
    if let Ok(v) = env::var("DBMCP_HTTP_AUTH_MODE") {
        cfg.server.http_auth.mode = parse_auth_mode(&v)?;
    }
    if let Ok(v) = env::var("DBMCP_HTTP_AUTH_TOKEN") {
        cfg.server.http_auth.token = v;
    }
    if let Ok(v) = env::var("DBMCP_HTTP_CORS_ALLOWED_ORIGINS") {
        cfg.server.http_cors_allowed_origins = v
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(str::to_string)
            .collect();
    }
    if let Ok(v) = env::var("DBMCP_TOOL_PROFILE") {
        cfg.server.tool_profile = parse_tool_profile(&v)?;
    }

    let env_connections = load_env_connections();
    if cfg.connections.is_empty() {
        cfg.connections = env_connections;
    } else if !env_connections.is_empty() {
        let mut by_name = cfg
            .connections
            .drain(..)
            .map(|c| (c.name.clone(), c))
            .collect::<BTreeMap<_, _>>();
        for conn in env_connections {
            by_name.insert(conn.name.clone(), conn);
        }
        cfg.connections = by_name.into_values().collect();
    }
    Ok(())
}

fn parse_transport(value: &str) -> Result<TransportMode> {
    match value.to_lowercase().as_str() {
        "stdio" => Ok(TransportMode::Stdio),
        "http" => Ok(TransportMode::Http),
        "both" => Ok(TransportMode::Both),
        other => bail!("invalid transport {other}"),
    }
}

fn parse_tool_profile(value: &str) -> Result<ToolProfile> {
    match value.to_lowercase().as_str() {
        "canonical" => Ok(ToolProfile::Canonical),
        "legacy" => Ok(ToolProfile::Legacy),
        "both" => Ok(ToolProfile::Both),
        other => {
            bail!("invalid tool profile {other:?}; expected \"canonical\", \"legacy\", or \"both\"")
        }
    }
}

fn load_env_connections() -> Vec<ConnectionConfig> {
    let mut out = vec![];

    if let Ok(dsn) = env::var("DBMCP_POSTGRES_DSN") {
        out.push(ConnectionConfig {
            name: env::var("DBMCP_POSTGRES_NAME").unwrap_or_else(|_| "postgres".to_string()),
            driver: "postgres".to_string(),
            postgres: Some(PostgresConfig {
                dsn,
                schema: env::var("DBMCP_POSTGRES_SCHEMA").unwrap_or_default(),
                read_only: env::var("DBMCP_POSTGRES_READ_ONLY")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(true),
            }),
            oracle: None,
            sqlite: None,
        });
    }

    if let Ok(dsn) = env::var("DBMCP_SQLITE_DSN").or_else(|_| env::var("SQLITE_URL")) {
        out.push(ConnectionConfig {
            name: env::var("DBMCP_SQLITE_NAME").unwrap_or_else(|_| "sqlite".to_string()),
            driver: "sqlite".to_string(),
            sqlite: Some(SQLiteConfig {
                dsn,
                read_only: env::var("DBMCP_SQLITE_READ_ONLY")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(true),
            }),
            oracle: None,
            postgres: None,
        });
    }

    if let Ok(jdbc_url) = env::var("DBMCP_ORACLE_JDBC_URL") {
        out.push(ConnectionConfig {
            name: env::var("DBMCP_ORACLE_NAME").unwrap_or_else(|_| "oracle".to_string()),
            driver: "oracle".to_string(),
            oracle: Some(OracleConfig {
                jdbc_url,
                username: env::var("DBMCP_ORACLE_USERNAME").unwrap_or_default(),
                password: env::var("DBMCP_ORACLE_PASSWORD").unwrap_or_default(),
                read_only: env::var("DBMCP_ORACLE_READ_ONLY")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(true),
            }),
            postgres: None,
            sqlite: None,
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http_config(
        transport: TransportMode,
        mode: HttpAuthMode,
        token: &str,
        addr: &str,
    ) -> Config {
        Config {
            server: ServerConfig {
                transport,
                http_addr: addr.to_string(),
                http_auth: HttpAuthConfig {
                    mode,
                    token: token.to_string(),
                },
                ..ServerConfig::default()
            },
            limits: LimitsConfig::default(),
            connections: vec![ConnectionConfig {
                name: "sqlite".to_string(),
                driver: "sqlite".to_string(),
                sqlite: Some(SQLiteConfig {
                    dsn: "sqlite::memory:".to_string(),
                    read_only: true,
                }),
                postgres: None,
                oracle: None,
            }],
        }
    }

    #[test]
    fn keeps_full_host_port_address() {
        assert_eq!(normalize_listen_addr("127.0.0.1:8080"), "127.0.0.1:8080");
        assert_eq!(normalize_listen_addr("0.0.0.0:8080"), "0.0.0.0:8080");
    }

    #[test]
    fn defaults_empty_host_to_wildcard() {
        assert_eq!(normalize_listen_addr(":8080"), "0.0.0.0:8080");
    }

    #[test]
    fn defaults_empty_address_to_default_port() {
        assert_eq!(normalize_listen_addr(""), "0.0.0.0:8080");
    }

    #[test]
    fn keeps_address_without_port() {
        assert_eq!(normalize_listen_addr("localhost"), "localhost");
    }

    #[test]
    fn http_transport_on_non_loopback_requires_auth() {
        let mut cfg = http_config(TransportMode::Http, HttpAuthMode::None, "", "0.0.0.0:8080");
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn http_transport_on_loopback_without_auth_is_allowed() {
        let mut cfg = http_config(
            TransportMode::Http,
            HttpAuthMode::None,
            "",
            "127.0.0.1:8080",
        );
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn stdio_transport_does_not_require_http_auth() {
        let mut cfg = http_config(TransportMode::Stdio, HttpAuthMode::None, "", "0.0.0.0:8080");
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn bearer_auth_requires_non_empty_token() {
        let mut cfg = http_config(
            TransportMode::Http,
            HttpAuthMode::Bearer,
            "",
            "127.0.0.1:8080",
        );
        assert!(cfg.validate().is_err());
        cfg.server.http_auth.token = "secret".to_string();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn bearer_token_is_trimmed_at_validation() {
        let mut cfg = http_config(
            TransportMode::Http,
            HttpAuthMode::Bearer,
            "  secret  ",
            "127.0.0.1:8080",
        );
        cfg.validate().expect("valid bearer config");
        assert_eq!(cfg.server.http_auth.token, "secret");
    }

    #[test]
    fn cors_origins_are_validated() {
        let mut cfg = http_config(
            TransportMode::Http,
            HttpAuthMode::Bearer,
            "t",
            "127.0.0.1:8080",
        );
        cfg.server.http_cors_allowed_origins = vec!["https://example.com".to_string()];
        assert!(cfg.validate().is_ok());
        cfg.server.http_cors_allowed_origins = vec!["https://exa\u{1}mple".to_string()];
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn parse_auth_mode_errors_on_unknown_values() {
        assert!(parse_auth_mode("bearer").is_ok());
        assert!(parse_auth_mode("none").is_ok());
        assert!(parse_auth_mode("beare").is_err());
        assert!(parse_auth_mode("basic").is_err());
    }

    #[test]
    fn default_tool_profile_is_canonical() {
        assert_eq!(ServerConfig::default().tool_profile, ToolProfile::Canonical);
        assert_eq!(
            Config::default().server.tool_profile,
            ToolProfile::Canonical
        );
    }

    #[test]
    fn parse_tool_profile_accepts_known_values() {
        assert_eq!(
            parse_tool_profile("canonical").expect("canonical"),
            ToolProfile::Canonical
        );
        assert_eq!(
            parse_tool_profile("Legacy").expect("legacy"),
            ToolProfile::Legacy
        );
        assert_eq!(parse_tool_profile("BOTH").expect("both"), ToolProfile::Both);
    }

    #[test]
    fn parse_tool_profile_rejects_unknown_values() {
        assert!(parse_tool_profile("bogus").is_err());
        assert!(parse_tool_profile("").is_err());
        assert!(parse_tool_profile("canonicall").is_err());
    }

    #[test]
    fn tool_profile_round_trips_through_config_json() {
        let parsed: ServerConfig = serde_json::from_str(r#"{"toolProfile":"legacy"}"#)
            .expect("parse server config with tool profile");
        assert_eq!(parsed.tool_profile, ToolProfile::Legacy);

        let rendered = serde_json::to_value(ServerConfig {
            tool_profile: ToolProfile::Both,
            ..ServerConfig::default()
        })
        .expect("serialize server config");
        assert_eq!(rendered["toolProfile"], "both");
    }

    #[test]
    fn unknown_tool_profile_in_config_json_is_rejected() {
        assert!(serde_json::from_str::<ServerConfig>(r#"{"toolProfile":"bogus"}"#).is_err());
    }

    #[test]
    fn output_format_parse_covers_known_and_rejects_unknown() {
        assert_eq!(
            OutputFormat::parse("json").expect("json"),
            OutputFormat::Json
        );
        assert_eq!(
            OutputFormat::parse("TEXT").expect("text"),
            OutputFormat::Text
        );
        assert!(OutputFormat::parse("yaml").is_err());
        assert_eq!(OutputFormat::default(), OutputFormat::Json);
        assert_eq!(OutputFormat::Json.as_str(), "json");
        assert_eq!(OutputFormat::Text.as_str(), "text");
    }

    #[test]
    fn shared_batch_limit_is_eight() {
        assert_eq!(MAX_BATCH_QUERIES, 8);
    }
}
