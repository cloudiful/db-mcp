use anyhow::{Context, Result, bail};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, fs, path::PathBuf, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum TransportMode {
    Stdio,
    Http,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HttpAuthMode {
    None,
    Bearer,
}

impl Default for HttpAuthMode {
    fn default() -> Self {
        Self::None
    }
}

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
            query_timeout: "15s".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub server: ServerConfig,
    pub limits: LimitsConfig,
    pub connections: Vec<ConnectionConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            limits: LimitsConfig::default(),
            connections: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConnectionConfig {
    pub name: String,
    pub driver: String,
    pub oracle: Option<OracleConfig>,
    pub postgres: Option<PostgresConfig>,
    pub sqlite: Option<SQLiteConfig>,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            driver: String::new(),
            oracle: None,
            postgres: None,
            sqlite: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct OracleConfig {
    pub jdbc_url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PostgresConfig {
    pub dsn: String,
    pub schema: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SQLiteConfig {
    pub dsn: String,
}

#[derive(Debug, Parser)]
pub struct Cli {
    #[arg(short = 'c', long, env = "DBMCP_CONFIG")]
    pub config: Option<PathBuf>,
    #[arg(long)]
    pub transport: Option<TransportMode>,
    #[arg(long = "http-addr")]
    pub http_addr: Option<String>,
    #[arg(long = "http-path")]
    pub http_path: Option<String>,
    #[arg(long)]
    pub debug: Option<bool>,
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

        apply_env(&mut cfg);

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
        } else if let Some(port) = http_addr.strip_prefix(':') {
            self.server.http_addr = format!("0.0.0.0:{port}");
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
            self.limits.query_timeout = "15s".to_string();
        }
        humantime::parse_duration(&self.limits.query_timeout)
            .context("invalid query_timeout")?;

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

fn apply_env(cfg: &mut Config) {
    if let Ok(v) = env::var("DBMCP_DEBUG") {
        if let Ok(parsed) = v.parse::<bool>() {
            cfg.server.debug = parsed;
        }
    }
    if let Ok(v) = env::var("DBMCP_TRANSPORT") {
        if let Ok(parsed) = parse_transport(&v) {
            cfg.server.transport = parsed;
        }
    }
    if let Ok(v) = env::var("DBMCP_HTTP_ADDR") {
        cfg.server.http_addr = v;
    }
    if let Ok(v) = env::var("DBMCP_HTTP_PATH") {
        cfg.server.http_path = v;
    }
    if let Ok(v) = env::var("DBMCP_HTTP_AUTH_MODE") {
        cfg.server.http_auth.mode = match v.to_lowercase().as_str() {
            "bearer" => HttpAuthMode::Bearer,
            _ => HttpAuthMode::None,
        };
    }
    if let Ok(v) = env::var("DBMCP_HTTP_AUTH_TOKEN") {
        cfg.server.http_auth.token = v;
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
}

fn parse_transport(value: &str) -> Result<TransportMode> {
    match value.to_lowercase().as_str() {
        "stdio" => Ok(TransportMode::Stdio),
        "http" => Ok(TransportMode::Http),
        "both" => Ok(TransportMode::Both),
        other => bail!("invalid transport {other}"),
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
            }),
            oracle: None,
            sqlite: None,
        });
    }

    if let Ok(dsn) = env::var("DBMCP_SQLITE_DSN").or_else(|_| env::var("SQLITE_URL")) {
        out.push(ConnectionConfig {
            name: env::var("DBMCP_SQLITE_NAME").unwrap_or_else(|_| "sqlite".to_string()),
            driver: "sqlite".to_string(),
            sqlite: Some(SQLiteConfig { dsn }),
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
            }),
            postgres: None,
            sqlite: None,
        });
    }

    out
}
