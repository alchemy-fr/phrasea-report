use std::env;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

pub struct Config {
    pub app_env: String,
    pub listen_port: u16,
    pub failed_logs_file: PathBuf,
    pub postgres: tokio_postgres::Config,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Self::from_lookup(|name| env::var(name).ok().filter(|value| !value.is_empty()))
    }

    fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let mut postgres = tokio_postgres::Config::new();
        postgres
            .host(get("POSTGRES_HOST").unwrap_or_else(|| "localhost".into()))
            .port(parse_port(get("POSTGRES_PORT"), "POSTGRES_PORT", 5432)?)
            .user(get("POSTGRES_USER").unwrap_or_default())
            .password(get("POSTGRES_PASSWORD").unwrap_or_default())
            .dbname(get("POSTGRES_DATABASE").unwrap_or_default())
            .application_name("phrasea-report")
            // Keeps the NOTICE of the idempotent schema creation out of the logs.
            .options("-c client_min_messages=warning")
            .connect_timeout(Duration::from_secs(5))
            // A peer gone without closing the connection (failover, network
            // partition) is detected within a minute instead of the 2-hour
            // keepalive default, during which the worker would wait on the insert.
            .tcp_user_timeout(Duration::from_secs(30))
            .keepalives_idle(Duration::from_secs(30))
            .keepalives_interval(Duration::from_secs(10))
            .keepalives_retries(3);

        Ok(Self {
            app_env: get("APP_ENV").unwrap_or_default(),
            listen_port: parse_port(get("LISTEN_PORT"), "LISTEN_PORT", 80)?,
            failed_logs_file: get("FAILED_LOGS_FILE")
                .unwrap_or_else(|| "failed_logs.jsonl".into())
                .into(),
            postgres,
        })
    }
}

fn parse_port(value: Option<String>, name: &str, default: u16) -> Result<u16> {
    value.map_or(Ok(default), |value| {
        value
            .parse()
            .with_context(|| format!("{name} is not a valid port: {value:?}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio_postgres::config::Host;

    fn config(vars: &[(&str, &str)]) -> Result<Config> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|name| vars.get(name).cloned())
    }

    #[test]
    fn reads_the_postgres_settings() {
        let config = config(&[
            ("POSTGRES_HOST", "db"),
            ("POSTGRES_PORT", "5433"),
            ("POSTGRES_USER", "report"),
            ("POSTGRES_PASSWORD", "p@ss word"),
            ("POSTGRES_DATABASE", "report"),
            ("APP_ENV", "prod"),
        ])
        .unwrap();

        assert_eq!(config.postgres.get_hosts(), [Host::Tcp("db".into())]);
        assert_eq!(config.postgres.get_ports(), [5433]);
        assert_eq!(config.postgres.get_user(), Some("report"));
        assert_eq!(config.postgres.get_password(), Some(&b"p@ss word"[..]));
        assert_eq!(config.postgres.get_dbname(), Some("report"));
        assert_eq!(config.app_env, "prod");
    }

    #[test]
    fn applies_the_defaults() {
        let config = config(&[]).unwrap();

        assert_eq!(config.listen_port, 80);
        assert_eq!(config.failed_logs_file, PathBuf::from("failed_logs.jsonl"));
        assert_eq!(config.postgres.get_ports(), [5432]);
    }

    #[test]
    fn detects_a_dead_connection_within_a_minute() {
        let config = config(&[]).unwrap();

        assert_eq!(
            config.postgres.get_tcp_user_timeout(),
            Some(&Duration::from_secs(30))
        );
        assert!(config.postgres.get_keepalives());
        assert_eq!(
            config.postgres.get_keepalives_idle(),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn rejects_an_invalid_port() {
        assert!(config(&[("POSTGRES_PORT", "abc")]).is_err());
        assert!(config(&[("LISTEN_PORT", "70000")]).is_err());
    }
}
