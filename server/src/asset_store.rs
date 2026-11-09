//! Single-owner immutable storage and bounded, pin-aware background reclamation.
use crate::{App, db, realtime::Grant, world::Registry};
use rusqlite::Connection;
use std::{
    collections::{HashMap, HashSet},
    fs::{File, OpenOptions, ReadDir},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};

const BATCH: usize = 64;
const MIN_AGE: Duration = Duration::from_secs(86400);

pub struct DataDirLock {
    _file: File,
}
impl DataDirLock {
    pub fn acquire(directory: &Path) -> anyhow::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(directory.join(".ssc.lock"))?;
        // Keep the inode and descriptor for the entire server lifetime. Never
        // unlink this lock file: doing so would allow competing lock inodes.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            anyhow::bail!(
                "Server data directory is already in use: {}",
                std::io::Error::last_os_error()
            );
        }
        Ok(Self { _file: file })
    }
}
impl Drop for DataDirLock {
    fn drop(&mut self) {
        // Logical ownership ends with this guard, not with a forked child's
        // transient pre-exec copy of the open file description. App retains
        // the guard until its last request/validation/GC owner has retired.
        let _ = unsafe { libc::flock(self._file.as_raw_fd(), libc::LOCK_UN) };
    }
}

pub fn live_versions(
    worlds: &Registry,
    grants: &HashMap<String, Grant>,
    now: i64,
) -> HashSet<String> {
    let mut pins = HashSet::new();
    for instance in worlds.instances.values() {
        pins.insert(instance.version.clone());
        for player in instance.players.values() {
            if let Some(version) = &player.avatar_version {
                pins.insert(version.clone());
            }
        }
    }
    for grant in grants.values().filter(|grant| grant.expires > now) {
        pins.insert(grant.version.clone());
    }
    pins
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn catalog_reference(db: &Connection, hash: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM assets WHERE version=?)",
        [hash],
        |row| row.get(0),
    )
}
fn remove_metadata(db: &Connection, hash: &str) -> rusqlite::Result<usize> {
    db.execute("DELETE FROM asset_versions WHERE hash=?1 AND NOT EXISTS(SELECT 1 FROM assets WHERE version=?1)",[hash])
}

#[derive(Default, Debug, serde::Serialize)]
pub struct Stats {
    pub scanned: usize,
    pub files: usize,
    pub rows: usize,
    pub bytes: u64,
    pub errors: usize,
}
#[derive(Default)]
pub struct Collector {
    cursor: Option<ReadDir>,
    metadata_after: String,
}
impl Collector {
    fn for_app(&mut self, app: &App) -> anyhow::Result<Stats> {
        let pins = live_versions(
            &app.worlds.lock().unwrap(),
            &app.grants.lock().unwrap(),
            db::now(),
        );
        self.pass(
            &app.config.data.join("assets"),
            &app.db.lock().unwrap(),
            &pins,
            SystemTime::now(),
            MIN_AGE,
        )
    }
    // Caller holds the publication/admission gate. No package bytes are read.
    // A persistent directory iterator prevents a large retained prefix from
    // starving obsolete files without storing an unbounded directory listing.
    pub fn pass(
        &mut self,
        directory: &Path,
        db: &Connection,
        pins: &HashSet<String>,
        now: SystemTime,
        min_age: Duration,
    ) -> anyhow::Result<Stats> {
        let mut stats = Stats::default();
        let deadline = Instant::now() + Duration::from_millis(25);
        if self.cursor.is_none() {
            self.cursor = Some(std::fs::read_dir(directory)?);
        }
        while stats.scanned < BATCH && Instant::now() < deadline {
            let Some(entry) = self.cursor.as_mut().unwrap().next() else {
                self.cursor = None;
                break;
            };
            stats.scanned += 1;
            let result: anyhow::Result<()> = (|| {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    return Ok(());
                }
                let name = entry.file_name();
                let Some(hash) = name
                    .to_str()
                    .and_then(|name| name.strip_suffix(".zip"))
                    .filter(|hash| valid_hash(hash))
                else {
                    return Ok(());
                };
                if pins.contains(hash) || catalog_reference(db, hash)? {
                    return Ok(());
                }
                let metadata = entry.metadata()?;
                if now
                    .duration_since(metadata.modified()?)
                    .map_or(true, |age| age < min_age)
                {
                    return Ok(());
                }
                // Delete metadata first. If unlink fails or the process stops,
                // the remaining unreferenced file is found on a later pass.
                stats.rows += remove_metadata(db, hash)?;
                std::fs::remove_file(entry.path())?;
                stats.files += 1;
                stats.bytes = stats.bytes.saturating_add(metadata.len());
                Ok(())
            })();
            if result.is_err() {
                stats.errors += 1;
            }
        }
        // Recover unreferenced metadata whose file is already missing. The
        // lexical cursor is bounded and remains stable across row deletion.
        if Instant::now() < deadline {
            let rows = {
                let mut statement = db.prepare(
                    "SELECT hash FROM asset_versions WHERE hash>? ORDER BY hash LIMIT 64",
                )?;
                statement
                    .query_map([&self.metadata_after], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?
            };
            if rows.is_empty() {
                self.metadata_after.clear();
            }
            for hash in rows {
                if Instant::now() >= deadline {
                    break;
                }
                self.metadata_after = hash.clone();
                if !valid_hash(&hash) || pins.contains(&hash) || catalog_reference(db, &hash)? {
                    continue;
                }
                match std::fs::symlink_metadata(directory.join(format!("{hash}.zip"))) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        stats.rows += remove_metadata(db, &hash)?
                    }
                    Err(_) => stats.errors += 1,
                    Ok(_) => (),
                }
            }
        }
        Ok(stats)
    }
}

pub fn start(app: Arc<App>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut collector = Collector::default();
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let gate = app.asset_gate.clone().lock_owned().await;
            let app = app.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                let _gate = gate;
                let result = collector.for_app(&app);
                (collector, result)
            })
            .await;
            match outcome {
                Ok((next, result)) => {
                    collector = next;
                    match result {
                        Ok(stats) if stats.files > 0 || stats.rows > 0 || stats.errors > 0 => {
                            eprintln!(
                                "Asset GC: {} files, {} bytes, {} rows removed; {} errors",
                                stats.files, stats.bytes, stats.rows, stats.errors
                            )
                        }
                        Ok(_) => (),
                        Err(error) => eprintln!("Asset GC deferred: {error}"),
                    }
                }
                Err(error) => {
                    eprintln!("Asset GC worker stopped: {error}");
                    break;
                }
            }
        }
    })
}

pub async fn collect_once(app: Arc<App>) -> anyhow::Result<Stats> {
    let gate = app.asset_gate.clone().lock_owned().await;
    tokio::task::spawn_blocking(move || {
        let _gate = gate;
        Collector::default().for_app(&app)
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::FileTimes, io::Read, os::unix::fs::symlink};

    fn database() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(include_str!("../migrations/001.sql"))
            .unwrap();
        db.execute("INSERT INTO users(id) VALUES('owner')", [])
            .unwrap();
        db
    }
    fn version(db: &Connection, hash: &str) {
        db.execute("INSERT INTO asset_versions VALUES(?,'{}',14)", [hash])
            .unwrap();
    }
    fn file(directory: &Path, hash: &str, modified: SystemTime) -> std::path::PathBuf {
        let path = directory.join(format!("{hash}.zip"));
        std::fs::write(&path, b"cached version").unwrap();
        File::open(&path)
            .unwrap()
            .set_times(FileTimes::new().set_modified(modified))
            .unwrap();
        path
    }
    #[test]
    fn exclusive_data_directory_lock_is_released_without_unlinking() {
        let directory = tempfile::tempdir().unwrap();
        let first = DataDirLock::acquire(directory.path()).unwrap();
        assert!(DataDirLock::acquire(directory.path()).is_err());
        let inherited = first._file.try_clone().unwrap();
        drop(first);
        let second = DataDirLock::acquire(directory.path()).unwrap();
        drop(inherited);
        assert!(
            DataDirLock::acquire(directory.path()).is_err(),
            "Closing an old descriptor must not unlock the new owner"
        );
        assert!(directory.path().join(".ssc.lock").is_file());
        drop(second);
        std::fs::remove_file(directory.path().join(".ssc.lock")).unwrap();
        let unrelated = directory.path().join("unrelated");
        std::fs::write(&unrelated, "keep").unwrap();
        symlink(&unrelated, directory.path().join(".ssc.lock")).unwrap();
        assert!(DataDirLock::acquire(directory.path()).is_err());
        assert_eq!(std::fs::read_to_string(unrelated).unwrap(), "keep");
    }
    #[test]
    fn collection_preserves_all_references_young_files_and_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let db = database();
        let now = SystemTime::now();
        let old = now - Duration::from_secs(172800);
        let hashes: Vec<_> = ["a", "b", "c", "d", "e", "f", "1", "2", "3", "4", "5"]
            .map(|letter| letter.repeat(64))
            .into();
        for hash in &hashes[..7] {
            version(&db, hash);
            file(
                directory.path(),
                hash,
                if hash == &hashes[6] { now } else { old },
            );
        }
        for (id, hash, enabled) in [
            ("current", &hashes[0], true),
            ("disabled", &hashes[1], false),
        ] {
            db.execute("INSERT INTO assets(id,owner,kind,name,version,created,enabled) VALUES(?,'owner','world','World',?,0,?)",rusqlite::params![id,hash,enabled]).unwrap();
        }
        file(directory.path(), &hashes[7], old); // Crash orphan without metadata.
        version(&db, &hashes[8]); // Metadata whose file is already missing.
        version(&db, &hashes[9]);
        let notes = directory.path().join("notes.txt");
        std::fs::write(&notes, "keep").unwrap();
        symlink(&notes, directory.path().join(format!("{}.zip", hashes[9]))).unwrap();
        std::fs::create_dir(directory.path().join(format!("{}.zip", hashes[10]))).unwrap();
        let pins: HashSet<_> = hashes[2..5].iter().cloned().collect();
        let mut in_flight =
            File::open(directory.path().join(format!("{}.zip", hashes[5]))).unwrap();
        let mut collector = Collector::default();
        let stats = collector
            .pass(directory.path(), &db, &pins, now, MIN_AGE)
            .unwrap();
        assert_eq!(stats.errors, 0);
        assert_eq!(stats.files, 2);
        assert_eq!(stats.rows, 2);
        assert_eq!(stats.bytes, 28);
        let mut downloaded = String::new();
        in_flight.read_to_string(&mut downloaded).unwrap();
        assert_eq!(
            downloaded, "cached version",
            "Opened download descriptors survive unlink"
        );
        for hash in &hashes[..5] {
            assert!(directory.path().join(format!("{hash}.zip")).is_file());
        }
        assert!(
            directory
                .path()
                .join(format!("{}.zip", hashes[6]))
                .is_file()
        );
        assert!(
            directory
                .path()
                .join(format!("{}.zip", hashes[9]))
                .is_symlink()
        );
        assert!(
            directory
                .path()
                .join(format!("{}.zip", hashes[10]))
                .is_dir()
        );
        assert_eq!(std::fs::read_to_string(&notes).unwrap(), "keep");
        db.execute("DELETE FROM assets", []).unwrap();
        let stats = collector
            .pass(directory.path(), &db, &HashSet::new(), now, MIN_AGE)
            .unwrap();
        assert_eq!(
            stats.files, 5,
            "Released catalog/live/grant pins become reclaimable"
        );
        assert!(
            directory
                .path()
                .join(format!("{}.zip", hashes[6]))
                .is_file()
        );
    }
    #[test]
    fn directory_cursor_is_bounded_and_does_not_starve_later_files() {
        let directory = tempfile::tempdir().unwrap();
        let db = database();
        let now = SystemTime::now();
        let old = now - Duration::from_secs(172800);
        for index in 0..200 {
            file(directory.path(), &format!("{index:064x}"), old);
        }
        let mut collector = Collector::default();
        let mut removed = 0;
        for _ in 0..32 {
            let stats = collector
                .pass(directory.path(), &db, &HashSet::new(), now, MIN_AGE)
                .unwrap();
            assert!(stats.scanned <= BATCH && stats.files <= BATCH);
            removed += stats.files;
            if removed == 200 {
                break;
            }
        }
        assert_eq!(removed, 200);
    }
    #[test]
    fn worlds_avatars_and_unexpired_grants_pin_immutable_versions() {
        let definition: crate::assets::Manifest = serde_json::from_value(
            serde_json::json!({"schema":1,"kind":"world","name":"Pins","scene":"world.glb"}),
        )
        .unwrap();
        let mut worlds = Registry::default();
        let world_hash = "a".repeat(64);
        let avatar_hash = "b".repeat(64);
        let pending_hash = "c".repeat(64);
        let id = worlds
            .admit(
                &crate::world::Join {
                    world: "world".into(),
                    send_divisor: 1,
                    receive_divisor: 1,
                    ..Default::default()
                },
                "a",
                "A",
                &world_hash,
                definition.clone(),
                vec![],
            )
            .unwrap();
        let player = worlds
            .instances
            .get_mut(&id)
            .unwrap()
            .players
            .get_mut("a")
            .unwrap();
        player.avatar = Some("avatar".into());
        player.avatar_version = Some(avatar_hash.clone());
        let pending = Grant {
            user: "pending".into(),
            session: "session".into(),
            name: "Pending".into(),
            join: crate::world::Join::default(),
            version: pending_hash.clone(),
            manifest: definition,
            sources: vec![],
            secret: vec![],
            expires: 101,
        };
        let mut expired = pending.clone();
        expired.expires = 99;
        expired.version = "d".repeat(64);
        let mut grants = HashMap::from([("pending".into(), pending), ("expired".into(), expired)]);
        assert_eq!(
            live_versions(&worlds, &grants, 100),
            HashSet::from([world_hash, avatar_hash, pending_hash.clone()])
        );
        worlds.leave("a");
        assert_eq!(
            live_versions(&worlds, &grants, 100),
            HashSet::from([pending_hash])
        );
        grants.clear();
        assert!(live_versions(&worlds, &grants, 100).is_empty());
    }
}
