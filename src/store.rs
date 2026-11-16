use std::error::Error as _;
use std::fmt;
use std::future::Future;
use std::io;
use std::time::Duration;

use anyhow::{Result, bail};
use tokio_postgres::error::SqlState;
use tokio_postgres::{Client, NoTls, Statement};
use tracing::{error, info, warn};

use crate::model::LogEntry;

const SCHEMA: &str = include_str!("structure.sql");

const INSERT: &str = "INSERT INTO logs(app_name, app_id, action, item, user_id, payload, event_date) \
     VALUES($1, $2, $3, $4, $5, $6, to_timestamp($7))";

const CONNECT_ATTEMPTS: u32 = 120;
const CONNECT_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum StoreError {
    /// The database cannot be reached: the entry is to be retried once reconnected.
    Connection(String),
    /// The database refused the entry.
    Rejected(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connection(message) | Self::Rejected(message) => f.write_str(message),
        }
    }
}

pub trait Store: Send {
    fn insert(&mut self, entry: &LogEntry) -> impl Future<Output = Result<(), StoreError>> + Send;

    /// Waits until the database is reachable again; an error means giving up.
    fn reconnect(&mut self) -> impl Future<Output = Result<()>> + Send;
}

pub struct PgStore {
    config: tokio_postgres::Config,
    client: Client,
    insert: Statement,
}

impl PgStore {
    /// Connects, retrying for 10 minutes, and creates the schema.
    pub async fn connect(config: tokio_postgres::Config) -> Result<Self> {
        let (client, insert) = connect_with_retries(&config).await?;
        Ok(Self {
            config,
            client,
            insert,
        })
    }
}

impl Store for PgStore {
    async fn insert(&mut self, entry: &LogEntry) -> Result<(), StoreError> {
        // to_timestamp() takes a double precision: exact up to 2^53 seconds.
        let event_date = entry.event_date as f64;
        self.client
            .execute(
                &self.insert,
                &[
                    &entry.app_name,
                    &entry.app_id,
                    &entry.action,
                    &entry.item,
                    &entry.user,
                    &entry.payload,
                    &event_date,
                ],
            )
            .await
            .map(|_| ())
            .map_err(|err| {
                if self.client.is_closed() || is_connection_error(&err) {
                    StoreError::Connection(describe(&err))
                } else {
                    StoreError::Rejected(describe(&err))
                }
            })
    }

    async fn reconnect(&mut self) -> Result<()> {
        (self.client, self.insert) = connect_with_retries(&self.config).await?;
        Ok(())
    }
}

/// The server's message, which the `Display` of the error leaves out.
fn describe(err: &tokio_postgres::Error) -> String {
    match err.as_db_error() {
        Some(db) => format!(
            "{}: {} (SQLSTATE {})",
            db.severity(),
            db.message(),
            db.code().code()
        ),
        None => match err.source() {
            Some(source) => format!("{err}: {source}"),
            None => err.to_string(),
        },
    }
}

fn is_connection_error(err: &tokio_postgres::Error) -> bool {
    if err.is_closed() || err.source().is_some_and(|source| source.is::<io::Error>()) {
        return true;
    }
    err.code().is_some_and(|code| {
        code.code().starts_with("08")
            || [
                SqlState::ADMIN_SHUTDOWN,
                SqlState::CRASH_SHUTDOWN,
                SqlState::CANNOT_CONNECT_NOW,
            ]
            .contains(code)
    })
}

async fn connect_with_retries(config: &tokio_postgres::Config) -> Result<(Client, Statement)> {
    for attempt in 1..=CONNECT_ATTEMPTS {
        match connect(config).await {
            Ok(connected) => {
                info!("Successfully connected!");
                return Ok(connected);
            }
            Err(err) => {
                warn!(
                    "Attempt {attempt}/{CONNECT_ATTEMPTS}: unable to connect to database: {err:#}"
                );
            }
        }
        if attempt < CONNECT_ATTEMPTS {
            tokio::time::sleep(CONNECT_DELAY).await;
        }
    }
    bail!("Unable to connect to database after {CONNECT_ATTEMPTS} attempts")
}

/// Connects, creates the schema if needed and prepares the insert.
async fn connect(config: &tokio_postgres::Config) -> Result<(Client, Statement)> {
    let (client, connection) = config.connect(NoTls).await?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            error!("Database connection lost: {}", describe(&err));
        }
    });
    client.batch_execute(SCHEMA).await?;
    let insert = client.prepare(INSERT).await?;
    Ok((client, insert))
}
