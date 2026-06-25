use anyhow::{Context, Result};

use crate::config;

#[derive(Debug, Clone)]
pub(super) enum OracleService {
    ServiceName(String),
    Sid(String),
}

#[derive(Debug, Clone)]
pub(super) struct OracleConnectSpec {
    pub host: String,
    pub port: u16,
    pub service: OracleService,
    pub username: String,
    pub password: String,
    pub target: String,
}

pub(super) fn parse_oracle_jdbc_url(cfg: &config::OracleConfig) -> Result<OracleConnectSpec> {
    let username = cfg.username.trim();
    let password = cfg.password.trim();
    if username.is_empty() || password.is_empty() {
        anyhow::bail!("oracle username and password are required");
    }

    let raw = cfg.jdbc_url.trim();
    let rest = raw
        .strip_prefix("jdbc:oracle:thin:@")
        .with_context(|| format!("unsupported oracle jdbc url {raw:?}"))?;
    let (host, port, service) = if let Some(service_part) = rest.strip_prefix("//") {
        let (host_port, service_name) = service_part
            .split_once('/')
            .context("oracle service JDBC URL must be jdbc:oracle:thin:@//host:port/service")?;
        let (host, port) = parse_host_port(host_port)?;
        (
            host,
            port,
            OracleService::ServiceName(service_name.to_string()),
        )
    } else {
        let parts = rest.split(':').collect::<Vec<_>>();
        if parts.len() != 3 {
            anyhow::bail!("oracle SID JDBC URL must be jdbc:oracle:thin:@host:port:SID");
        }
        let port = parts[1]
            .parse::<u16>()
            .with_context(|| format!("invalid oracle jdbc port {:?}", parts[1]))?;
        (
            parts[0].to_string(),
            port,
            OracleService::Sid(parts[2].to_string()),
        )
    };

    if host.trim().is_empty() {
        anyhow::bail!("oracle jdbc host is empty");
    }
    match &service {
        OracleService::ServiceName(v) | OracleService::Sid(v) if v.trim().is_empty() => {
            anyhow::bail!("oracle jdbc service/SID is empty")
        }
        _ => {}
    }

    Ok(OracleConnectSpec {
        host,
        port,
        service,
        username: username.to_string(),
        password: password.to_string(),
        target: raw.to_string(),
    })
}

fn parse_host_port(value: &str) -> Result<(String, u16)> {
    let (host, port) = value
        .rsplit_once(':')
        .context("oracle jdbc host:port is required")?;
    let port = port
        .parse::<u16>()
        .with_context(|| format!("invalid oracle jdbc port {port:?}"))?;
    Ok((host.to_string(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(url: &str) -> config::OracleConfig {
        config::OracleConfig {
            jdbc_url: url.to_string(),
            username: "readuser".to_string(),
            password: "secret".to_string(),
            read_only: true,
        }
    }

    #[test]
    fn parses_sid_jdbc_url() {
        let spec = parse_oracle_jdbc_url(&cfg("jdbc:oracle:thin:@localhost:1521:XE")).unwrap();
        assert_eq!(spec.host, "localhost");
        assert_eq!(spec.port, 1521);
        assert!(matches!(spec.service, OracleService::Sid(ref sid) if sid == "XE"));
    }

    #[test]
    fn parses_service_jdbc_url() {
        let spec = parse_oracle_jdbc_url(&cfg("jdbc:oracle:thin:@//db.example.com:1521/FREEPDB1"))
            .unwrap();
        assert_eq!(spec.host, "db.example.com");
        assert_eq!(spec.port, 1521);
        assert!(
            matches!(spec.service, OracleService::ServiceName(ref service) if service == "FREEPDB1")
        );
    }

    #[test]
    fn rejects_invalid_jdbc_url() {
        assert!(parse_oracle_jdbc_url(&cfg("oracle://localhost/XE")).is_err());
    }

    #[test]
    fn rejects_empty_credentials() {
        let mut cfg = cfg("jdbc:oracle:thin:@localhost:1521:XE");
        cfg.password.clear();
        assert!(parse_oracle_jdbc_url(&cfg).is_err());
    }
}
