//! S07-x: Raft snapshot file format + storage.
//!
//! Snapshots are written to `<dir>/snap-<index>.bin` and
//! capture `(current_term, voted_for, log[..=last_included_index])`.
//! They are loaded at boot BEFORE replaying the WAL, so
//! the WAL only needs to contain entries after the
//! snapshot's `last_included_index`. Without snapshots
//! the WAL grows unboundedly.

use std::fs::{self, File};
use std::io::{Error, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::types::{LogEntry, NodeId};

/// Monotonic counter used to make in-flight
/// snapshot tmp filenames unique across Nodes
/// that share a snapshot directory. Without
/// this, concurrent writes from multiple
/// Raft peers race on `<dir>/snap-<idx>.bin.tmp`
/// and the loser of the race sees `NotFound`
/// at `rename` time.
static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

const MAGIC: &[u8; 8] = b"BEERSNP1";

#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub last_included_index: u64,
    pub last_included_term: u64,
    pub current_term: u64,
    pub voted_for: Option<NodeId>,
    /// Entries `log[..=last_included_index]` — the entire
    /// log up to and including the snapshot point. Kept
    /// so a fresh boot can re-apply these into the KV
    /// state machine without re-reading the WAL.
    pub log: Vec<LogEntry>,
}

#[derive(Debug)]
pub struct SnapshotStore {
    dir: PathBuf,
}

impl SnapshotStore {
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// List every snapshot file in `dir`, sorted by
    /// `last_included_index` (ascending). Files that
    /// don't parse are silently skipped — a corrupted
    /// snapshot should not prevent the cluster from
    /// booting (the next-newest good snapshot wins).
    pub fn list(&self) -> std::io::Result<Vec<u64>> {
        let mut out = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("snap-") || !name.ends_with(".bin") {
                continue;
            }
            let middle = &name[5..name.len() - 4];
            if let Ok(index) = middle.parse::<u64>() {
                out.push(index);
            }
        }
        out.sort_unstable();
        Ok(out)
    }

    /// Pick the most recent snapshot file in `dir`
    /// (highest `last_included_index`). Returns `Ok(None)`
    /// if the directory has no snapshot files.
    pub fn latest(&self) -> std::io::Result<Option<Snapshot>> {
        let mut indices = self.list()?;
        let Some(latest_index) = indices.pop() else {
            return Ok(None);
        };
        match self.read(latest_index) {
            Ok(snap) => Ok(Some(snap)),
            Err(error) => {
                // Fall back to the next-newest valid
                // snapshot if the newest one is
                // corrupted.
                while let Some(idx) = indices.pop() {
                    if let Ok(snap) = self.read(idx) {
                        return Ok(Some(snap));
                    }
                    let _ = error;
                }
                Err(error)
            }
        }
    }

    /// Atomically write a snapshot: serialize to
    /// `<dir>/snap-<index>.tmp`, fsync, rename to
    /// `<dir>/snap-<index>.bin`. After a successful
    /// write, older snapshots are deleted — we keep
    /// at most the 2 most recent so a crash mid-write
    /// leaves us with a fallback.
    pub fn write(&self, snap: &Snapshot) -> std::io::Result<()> {
        let final_path = self.path_for(snap.last_included_index);
        // Use a process-unique counter to
        // disambiguate concurrent writes from
        // multiple Nodes that share the same
        // snapshot dir.
        let counter = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = {
            let mut p = final_path.as_os_str().to_owned();
            p.push(format!(".tmp.{counter}"));
            PathBuf::from(p)
        };
        let _ = fs::create_dir_all(&self.dir);
        let payload = bincode::serialize(snap)
            .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
        let length = u32::try_from(payload.len())
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "snapshot too large"))?;
        {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_path)?;
            f.write_all(MAGIC)?;
            f.write_all(&length.to_le_bytes())?;
            f.write_all(&payload)?;
            f.sync_all()?;
        }
        fs::rename(&tmp_path, &final_path)?;
        // Note: we do NOT clean up stale
        // `.tmp.*` files here. A racy sweep
        // would delete another Node's
        // in-flight tmp file and crash their
        // rename. Stale tmp files are harmless
        // (they are not visible to `list()`
        // because they don't match
        // `snap-<N>.bin`) and can be cleaned up
        // out-of-band if desired.
        let mut existing = self.list()?;
        while existing.len() > 2 {
            let drop = existing.remove(0);
            let _ = fs::remove_file(self.path_for(drop));
        }
        Ok(())
    }

    pub fn read(&self, last_included_index: u64) -> std::io::Result<Snapshot> {
        let path = self.path_for(last_included_index);
        let mut f = File::open(&path)?;
        let mut magic = [0_u8; 8];
        f.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "invalid Raft snapshot magic",
            ));
        }
        let mut length = [0_u8; 4];
        f.read_exact(&mut length)?;
        let length = u32::from_le_bytes(length) as usize;
        let mut payload = vec![0_u8; length];
        f.read_exact(&mut payload)?;
        let snap: Snapshot = bincode::deserialize(&payload)
            .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
        Ok(snap)
    }

    /// Like `read` but returns `Ok(None)` when the
    /// file doesn't exist (used by snapshot
    /// composition during incremental
    /// snapshots).
    pub fn read_opt(
        &self,
        last_included_index: u64,
    ) -> std::io::Result<Option<Snapshot>> {
        let path = self.path_for(last_included_index);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(self.read(last_included_index)?))
    }

    fn path_for(&self, index: u64) -> PathBuf {
        self.dir.join(format!("snap-{index}.bin"))
    }
}

/// Tiny shim around `OpenOptions::new().create(true).write(true).truncate(true)`
/// kept for documentation; the inline form is used in
/// `SnapshotStore::write` directly.
#[allow(dead_code)]
struct OpenOptionsCreateWrite;

#[allow(dead_code)]
impl OpenOptionsCreateWrite {
    fn new() -> fs::OpenOptions {
        let mut o = fs::OpenOptions::new();
        o.create(true).write(true).truncate(true);
        o
    }
}