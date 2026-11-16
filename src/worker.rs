use anyhow::Result;
use tokio::sync::mpsc::Receiver;
use tracing::{error, info, warn};

use crate::failed_logs::FailedLogs;
use crate::model::LogEntry;
use crate::store::{Store, StoreError};

/// Writes the queued entries one at a time, the only user of the store.
pub struct Worker<S> {
    store: S,
    failed: FailedLogs,
}

impl<S: Store> Worker<S> {
    pub fn new(store: S, failed: FailedLogs) -> Self {
        Self { store, failed }
    }

    /// Runs until the queue is closed and drained; an error means the
    /// database could not be reached again.
    pub async fn run(&mut self, queue: &mut Receiver<LogEntry>) -> Result<()> {
        self.replay_failed().await;
        while let Some(entry) = queue.recv().await {
            self.persist(entry).await?;
        }
        info!("Log queue drained");
        Ok(())
    }

    async fn persist(&mut self, entry: LogEntry) -> Result<()> {
        loop {
            match self.store.insert(&entry).await {
                Ok(()) => return Ok(()),
                Err(StoreError::Connection(err)) => {
                    warn!("Database unreachable, reconnecting: {err}");
                    self.store.reconnect().await?;
                    self.replay_failed().await;
                }
                Err(StoreError::Rejected(err)) => {
                    error!("Failed to persist log (non-connection error): {err}");
                    if let Err(err) = self.failed.append(&entry) {
                        error!("Unable to write failed log: {err}");
                    }
                    return Ok(());
                }
            }
        }
    }

    /// Inserts the stored failed entries again, keeping those still refused.
    async fn replay_failed(&mut self) {
        let entries = match self.failed.read() {
            Ok(entries) => entries,
            Err(err) => {
                error!("Unable to read failed logs file: {err}");
                return;
            }
        };
        if entries.is_empty() {
            return;
        }

        let total = entries.len();
        let mut remaining = Vec::new();
        for entry in entries {
            if let Err(err) = self.store.insert(&entry).await {
                warn!("Replay failed for log: {err}");
                remaining.push(entry);
            }
        }
        info!(
            "Replayed {} of {total} failed logs",
            total - remaining.len()
        );
        if let Err(err) = self.failed.replace(&remaining) {
            error!("Unable to rewrite failed logs file: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;
    use std::collections::VecDeque;
    use tokio::sync::mpsc;

    /// Answers the inserts from `outcomes` (then succeeds), records the
    /// inserted entries.
    #[derive(Default)]
    struct FakeStore {
        outcomes: VecDeque<Result<(), StoreError>>,
        inserted: Vec<LogEntry>,
        reconnects: usize,
        reconnect_fails: bool,
    }

    impl Store for FakeStore {
        async fn insert(&mut self, entry: &LogEntry) -> Result<(), StoreError> {
            let outcome = self.outcomes.pop_front().unwrap_or(Ok(()));
            if outcome.is_ok() {
                self.inserted.push(entry.clone());
            }
            outcome
        }

        async fn reconnect(&mut self) -> Result<()> {
            self.reconnects += 1;
            if self.reconnect_fails {
                bail!("gave up");
            }
            Ok(())
        }
    }

    fn entry(action: &str) -> LogEntry {
        LogEntry {
            action: action.into(),
            ..LogEntry::default()
        }
    }

    fn connection() -> Result<(), StoreError> {
        Err(StoreError::Connection("closed".into()))
    }

    fn rejected() -> Result<(), StoreError> {
        Err(StoreError::Rejected("value too long".into()))
    }

    async fn run(
        store: FakeStore,
        failed: FailedLogs,
        entries: &[LogEntry],
    ) -> (Result<()>, FakeStore) {
        let (tx, mut rx) = mpsc::channel(entries.len().max(1));
        for entry in entries {
            tx.try_send(entry.clone()).unwrap();
        }
        drop(tx);
        let mut worker = Worker::new(store, failed);
        let result = worker.run(&mut rx).await;
        (result, worker.store)
    }

    #[tokio::test]
    async fn inserts_every_queued_entry_in_order() {
        let dir = tempfile::tempdir().unwrap();

        let (result, store) = run(
            FakeStore::default(),
            FailedLogs::new(dir.path().join("failed.jsonl")),
            &[entry("a"), entry("b")],
        )
        .await;

        result.unwrap();
        assert_eq!(store.inserted, [entry("a"), entry("b")]);
    }

    #[tokio::test]
    async fn retries_the_entry_after_reconnecting() {
        let dir = tempfile::tempdir().unwrap();
        let store = FakeStore {
            outcomes: [connection(), connection()].into(),
            ..FakeStore::default()
        };

        let (result, store) = run(
            store,
            FailedLogs::new(dir.path().join("failed.jsonl")),
            &[entry("a")],
        )
        .await;

        result.unwrap();
        assert_eq!(store.reconnects, 2);
        assert_eq!(store.inserted, [entry("a")]);
    }

    #[tokio::test]
    async fn stores_a_rejected_entry_and_goes_on() {
        let dir = tempfile::tempdir().unwrap();
        let failed = dir.path().join("failed.jsonl");
        let store = FakeStore {
            outcomes: [rejected()].into(),
            ..FakeStore::default()
        };

        let (result, store) = run(store, FailedLogs::new(&failed), &[entry("a"), entry("b")]).await;

        result.unwrap();
        assert_eq!(store.inserted, [entry("b")]);
        assert_eq!(FailedLogs::new(&failed).read().unwrap(), [entry("a")]);
    }

    #[tokio::test]
    async fn replays_the_failed_entries_at_start_and_after_a_reconnection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failed.jsonl");
        let failed = FailedLogs::new(&path);
        failed.append(&entry("old")).unwrap();
        let store = FakeStore {
            // The start replay of "old" is refused again, then "new" meets a
            // lost connection: "old" is replayed after the reconnection.
            outcomes: [rejected(), connection()].into(),
            ..FakeStore::default()
        };

        let (result, store) = run(store, failed, &[entry("new")]).await;

        result.unwrap();
        assert_eq!(store.inserted, [entry("old"), entry("new")]);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn keeps_the_entries_refused_again_on_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failed.jsonl");
        let failed = FailedLogs::new(&path);
        failed.append(&entry("a")).unwrap();
        failed.append(&entry("b")).unwrap();
        let store = FakeStore {
            outcomes: [Ok(()), rejected()].into(),
            ..FakeStore::default()
        };

        let (result, store) = run(store, failed, &[]).await;

        result.unwrap();
        assert_eq!(store.inserted, [entry("a")]);
        assert_eq!(FailedLogs::new(&path).read().unwrap(), [entry("b")]);
    }

    #[tokio::test]
    async fn fails_when_the_database_cannot_be_reached_again() {
        let dir = tempfile::tempdir().unwrap();
        let store = FakeStore {
            outcomes: [connection()].into(),
            reconnect_fails: true,
            ..FakeStore::default()
        };

        let (result, store) = run(
            store,
            FailedLogs::new(dir.path().join("failed.jsonl")),
            &[entry("a")],
        )
        .await;

        assert!(result.is_err());
        assert!(store.inserted.is_empty());
    }
}
