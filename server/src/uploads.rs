//! Bounded upload admission and RAII staging, including cancelled HTTP futures.
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

pub const MAX_PACKAGE_BYTES: u64 = 1_000_000_000;
#[derive(Clone, Copy, Default, serde::Serialize)]
pub struct Usage {
    pub active: usize,
    pub reserved_bytes: u64,
}
pub struct Budget {
    usage: Mutex<Usage>,
    max_active: usize,
    max_bytes: u64,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            usage: Mutex::default(),
            max_active: 2,
            max_bytes: 2 * MAX_PACKAGE_BYTES,
        }
    }
}
pub struct Lease {
    budget: Arc<Budget>,
    bytes: u64,
}
impl Budget {
    pub fn reserve(self: &Arc<Self>, bytes: u64) -> Result<Lease, &'static str> {
        let mut usage = self.usage.lock().unwrap();
        if bytes > MAX_PACKAGE_BYTES
            || usage.active >= self.max_active
            || usage.reserved_bytes.saturating_add(bytes) > self.max_bytes
        {
            return Err("Upload resources busy; retry after current uploads finish");
        }
        usage.active += 1;
        usage.reserved_bytes += bytes;
        Ok(Lease {
            budget: self.clone(),
            bytes,
        })
    }
    pub fn usage(&self) -> Usage {
        *self.usage.lock().unwrap()
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut usage = self.budget.usage.lock().unwrap();
        usage.active -= 1;
        usage.reserved_bytes -= self.bytes;
    }
}
pub struct Stage {
    path: PathBuf,
    _lease: Lease,
}
impl Stage {
    pub fn create(directory: &Path, lease: Lease) -> std::io::Result<(Self, File)> {
        let path = directory.join(format!("upload-{}.tmp", crate::db::token()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        Ok((
            Self {
                path,
                _lease: lease,
            },
            file,
        ))
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("Upload staging cleanup deferred: {error}");
        }
    }
}

fn managed(name: &str) -> bool {
    name.strip_prefix("upload-")
        .and_then(|name| name.strip_suffix(".tmp"))
        .is_some_and(|token| {
            token.len() == 64
                && token
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}
// Only call at startup, after the exclusive data-directory lock and before
// serving HTTP. There can be no active uploader whose file would be removed.
pub fn recover(directory: &Path) -> std::io::Result<(usize, u64)> {
    let (mut count, mut bytes) = (0, 0u64);
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() || !entry.file_name().to_str().is_some_and(managed) {
            continue;
        }
        let size = entry.metadata()?.len();
        std::fs::remove_file(entry.path())?;
        count += 1;
        bytes = bytes.saturating_add(size);
    }
    Ok((count, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, os::unix::fs::symlink};
    #[test]
    fn reservations_bound_concurrency_and_bytes_and_drop_releases_them() {
        let budget = Arc::new(Budget {
            usage: Mutex::default(),
            max_active: 2,
            max_bytes: 10,
        });
        let first = budget.reserve(8).unwrap();
        assert!(budget.reserve(3).is_err());
        let second = budget.reserve(2).unwrap();
        assert!(budget.reserve(0).is_err());
        assert_eq!(budget.usage().reserved_bytes, 10);
        drop(first);
        drop(second);
        assert_eq!(budget.usage().active, 0);
        assert_eq!(budget.usage().reserved_bytes, 0);
    }
    #[test]
    fn stage_drop_cleans_partial_but_never_removes_a_published_file() {
        let directory = tempfile::tempdir().unwrap();
        let budget = Arc::new(Budget::default());
        let (stage, mut file) =
            Stage::create(directory.path(), budget.reserve(100).unwrap()).unwrap();
        file.write_all(b"partial").unwrap();
        let path = stage.path().to_path_buf();
        drop(stage);
        assert!(!path.exists());
        assert_eq!(budget.usage().active, 0);
        drop(file);
        let (stage, mut file) =
            Stage::create(directory.path(), budget.reserve(100).unwrap()).unwrap();
        file.write_all(b"published").unwrap();
        drop(file);
        let destination = directory.path().join("published.zip");
        std::fs::rename(stage.path(), &destination).unwrap();
        drop(stage);
        assert_eq!(std::fs::read(destination).unwrap(), b"published");
        assert_eq!(budget.usage().reserved_bytes, 0);
    }
    #[tokio::test]
    async fn cancelled_validation_waiter_does_not_release_a_running_worker_lease() {
        let directory = tempfile::tempdir().unwrap();
        let budget = Arc::new(Budget::default());
        let (stage, file) = Stage::create(directory.path(), budget.reserve(100).unwrap()).unwrap();
        drop(file);
        let path = stage.path().to_path_buf();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let task = tokio::task::spawn_blocking(move || {
            let stage = stage;
            started_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            drop(stage);
            finished_tx.send(()).unwrap();
        });
        started_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        task.abort();
        drop(task);
        assert_eq!(budget.usage().active, 1);
        assert!(path.exists());
        release_tx.send(()).unwrap();
        finished_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(budget.usage().active, 0);
        assert!(!path.exists());
    }
    #[test]
    fn exclusive_startup_recovery_only_removes_owned_regular_staging_files() {
        let directory = tempfile::tempdir().unwrap();
        for token in ["a".repeat(64), "b".repeat(64)] {
            std::fs::write(directory.path().join(format!("upload-{token}.tmp")), "old").unwrap();
        }
        let notes = directory.path().join("notes.tmp");
        std::fs::write(&notes, "keep").unwrap();
        let link = directory
            .path()
            .join(format!("upload-{}.tmp", "c".repeat(64)));
        symlink(&notes, &link).unwrap();
        let nested = directory
            .path()
            .join(format!("upload-{}.tmp", "d".repeat(64)));
        std::fs::create_dir(&nested).unwrap();
        let invalid = directory
            .path()
            .join(format!("upload-{}.tmp", "A".repeat(64)));
        std::fs::write(&invalid, "keep").unwrap();
        assert_eq!(recover(directory.path()).unwrap(), (2, 6));
        assert!(link.is_symlink() && nested.is_dir() && invalid.is_file());
        assert_eq!(std::fs::read_to_string(notes).unwrap(), "keep");
    }
}
