use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

use tracing::warn;

use crate::model::LogEntry;

/// The entries the database refused for another reason than a lost
/// connection, one JSON object per line, replayed after each reconnection.
pub struct FailedLogs {
    path: PathBuf,
}

impl FailedLogs {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn append(&self, entry: &LogEntry) -> io::Result<()> {
        let mut line = serde_json::to_vec(entry)?;
        line.push(b'\n');
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?
            .write_all(&line)
    }

    /// The stored entries; corrupt lines are logged and dropped.
    pub fn read(&self) -> io::Result<Vec<LogEntry>> {
        // Bytes, not a string: invalid UTF-8 (a line cut by a crash) only
        // spoils its own line.
        let data = match fs::read(&self.path) {
            Ok(data) => data,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err),
        };

        Ok(data
            .split(|&byte| byte == b'\n')
            .filter(|line| !line.trim_ascii().is_empty())
            .filter_map(|line| {
                serde_json::from_slice(line)
                    .inspect_err(|err| warn!("Corrupt failed log entry: {err}"))
                    .ok()
            })
            .collect())
    }

    /// Replaces the stored entries; no entry left removes the file.
    pub fn replace(&self, entries: &[LogEntry]) -> io::Result<()> {
        if entries.is_empty() {
            return match fs::remove_file(&self.path) {
                Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
                _ => Ok(()),
            };
        }

        let mut data = Vec::new();
        for entry in entries {
            serde_json::to_writer(&mut data, entry)?;
            data.push(b'\n');
        }
        fs::write(&self.path, data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(action: &str) -> LogEntry {
        LogEntry {
            action: action.into(),
            ..LogEntry::default()
        }
    }

    #[test]
    fn reads_nothing_without_a_file() {
        let dir = tempfile::tempdir().unwrap();

        let failed = FailedLogs::new(dir.path().join("failed.jsonl"));

        assert!(failed.read().unwrap().is_empty());
        failed.replace(&[]).unwrap();
    }

    #[test]
    fn appends_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let failed = FailedLogs::new(dir.path().join("failed.jsonl"));

        failed.append(&entry("a")).unwrap();
        failed.append(&entry("b")).unwrap();

        assert_eq!(failed.read().unwrap(), [entry("a"), entry("b")]);
    }

    #[test]
    fn skips_corrupt_and_blank_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failed.jsonl");
        fs::write(
            &path,
            "{\"action\":\"a\"}\nnot json\n\n{\"action\":\"b\"}\n",
        )
        .unwrap();

        assert_eq!(
            FailedLogs::new(path).read().unwrap(),
            [entry("a"), entry("b")]
        );
    }

    #[test]
    fn skips_a_line_with_invalid_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failed.jsonl");
        fs::write(
            &path,
            b"{\"action\":\"a\"}\n{\"action\":\"\xc3\n{\"action\":\"b\"}\n",
        )
        .unwrap();

        assert_eq!(
            FailedLogs::new(path).read().unwrap(),
            [entry("a"), entry("b")]
        );
    }

    #[test]
    fn replaces_the_entries_and_removes_the_file_once_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failed.jsonl");
        let failed = FailedLogs::new(&path);
        failed.append(&entry("a")).unwrap();
        failed.append(&entry("b")).unwrap();

        failed.replace(&[entry("b")]).unwrap();
        assert_eq!(failed.read().unwrap(), [entry("b")]);

        failed.replace(&[]).unwrap();
        assert!(!path.exists());
    }
}
