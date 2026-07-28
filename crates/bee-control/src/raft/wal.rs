use std::fs::{File, OpenOptions};
use std::io::{Error, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::{LogEntry, NodeId, Term};

const MAGIC: &[u8; 8] = b"BEERAWL1";

#[derive(Debug, Serialize, Deserialize)]
enum WalRecord {
    Entry {
        /// S07-x: the entry's global Raft log index
        /// (1-based, survives log truncation and
        /// snapshots). Before S07-x the WAL did not
        /// carry the index — the global index was
        /// implicit as `state.log.len()`. With
        /// snapshotting, the in-memory log can be
        /// truncated and the index must be persisted
        /// alongside the entry so replay can
        /// reconstruct the correct global ordering.
        index: u64,
        entry: LogEntry,
    },
    TermAndVote {
        term: Term,
        voted_for: Option<NodeId>,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WalReplay {
    pub entries: Vec<WalEntry>,
    pub term: Term,
    pub voted_for: Option<NodeId>,
}

/// S07-x: a WAL entry paired with its global Raft log
/// index. Replaces the raw `LogEntry` list in
/// `WalReplay` so callers know the absolute position
/// of every entry, even after the in-memory log has
/// been compacted by a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalEntry {
    pub index: u64,
    pub entry: LogEntry,
}

#[derive(Debug)]
pub struct RaftLogWal {
    file: File,
    path: PathBuf,
}

impl RaftLogWal {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)?;
        if file.metadata()?.len() == 0 {
            file.write_all(MAGIC)?;
            file.sync_all()?;
        }
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    /// S07-x: append a `LogEntry` with its global
    /// `index`. Replaces the index-less `append`
    /// from S07. The on-disk format changed
    /// (BREAKING) — only acceptable because the WAL
    /// has not been shipped to anyone yet.
    pub fn append(&mut self, index: u64, entry: &LogEntry) -> std::io::Result<()> {
        self.write_record(&WalRecord::Entry {
            index,
            entry: entry.clone(),
        })
    }

    pub fn persist_term_and_vote(
        &mut self,
        term: Term,
        voted_for: Option<NodeId>,
    ) -> std::io::Result<()> {
        self.write_record(&WalRecord::TermAndVote { term, voted_for })
    }

    /// S07-x: replay the WAL. Entries with
    /// `index <= skip_through` are discarded
    /// (they are already covered by a snapshot).
    /// `skip_through` is the
    /// `last_included_index` from the snapshot
    /// (`0` if there is no snapshot — every
    /// entry is kept).
    pub fn replay(
        path: &Path,
        skip_through: u64,
    ) -> std::io::Result<WalReplay> {
        let mut file = File::open(path)?;
        let mut magic = [0_u8; 8];
        file.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(Error::new(ErrorKind::InvalidData, "invalid Raft WAL magic"));
        }

        let mut replay = WalReplay::default();
        loop {
            let mut length = [0_u8; 4];
            match file.read_exact(&mut length) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(error),
            }
            let length = u32::from_le_bytes(length) as usize;
            let mut payload = vec![0_u8; length];
            file.read_exact(&mut payload)?;
            let record: WalRecord = bincode::deserialize(&payload)
                .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
            match record {
                WalRecord::Entry { index, entry } => {
                    if index > skip_through {
                        replay.entries.push(WalEntry { index, entry });
                    }
                }
                WalRecord::TermAndVote { term, voted_for } => {
                    replay.term = term;
                    replay.voted_for = voted_for;
                }
            }
        }
        Ok(replay)
    }

    pub fn sync(&mut self) -> std::io::Result<()> {
        self.file.sync_all()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// S07-x: rewrite the WAL so it only contains
    /// entries with `index >= keep_from_index`.
    /// Used after a snapshot to drop the entries
    /// that are now covered by the snapshot.
    ///
    /// Implementation: read every WAL record, write
    /// the surviving ones to a `.tmp` sibling, then
    /// atomically rename. The magic header is
    /// re-emitted at the start of the new file. If
    /// no records survive, the WAL is left empty
    /// (just the magic header — same shape as a
    /// freshly-opened WAL).
    pub fn truncate_before(
        &mut self,
        keep_from_index: u64,
    ) -> std::io::Result<()> {
        let path = self.path.clone();
        let tmp_path = {
            let mut p = path.as_os_str().to_owned();
            p.push(".truncating");
            PathBuf::from(p)
        };
        // Read everything currently on disk.
        let replay = Self::replay(&path, 0)?;
        // Drop the current file handle so we can
        // rename over it.
        let mut tmp = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .read(true)
            .open(&tmp_path)?;
        tmp.write_all(MAGIC)?;
        for wal_entry in &replay.entries {
            if wal_entry.index >= keep_from_index {
                let payload = bincode::serialize(&WalRecord::Entry {
                    index: wal_entry.index,
                    entry: wal_entry.entry.clone(),
                })
                .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
                let length = u32::try_from(payload.len())
                    .map_err(|_| Error::new(ErrorKind::InvalidInput, "Raft WAL record too large"))?;
                tmp.write_all(&length.to_le_bytes())?;
                tmp.write_all(&payload)?;
            }
        }
        // Re-emit the most recent TermAndVote record.
        tmp.write_all(&{
            let payload = bincode::serialize(&WalRecord::TermAndVote {
                term: replay.term,
                voted_for: replay.voted_for,
            })
            .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
            let length = u32::try_from(payload.len())
                .map_err(|_| Error::new(ErrorKind::InvalidInput, "Raft WAL record too large"))?;
            let mut v = length.to_le_bytes().to_vec();
            v.extend_from_slice(&payload);
            v
        })?;
        tmp.sync_all()?;
        drop(tmp);
        // Replace the live file with the rebuilt
        // one, then reopen so subsequent appends go
        // to the new file.
        std::fs::rename(&tmp_path, &path)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        self.file = file;
        Ok(())
    }

    fn write_record(&mut self, record: &WalRecord) -> std::io::Result<()> {
        let payload = bincode::serialize(record)
            .map_err(|error| Error::new(ErrorKind::InvalidData, error))?;
        let length = u32::try_from(payload.len())
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "Raft WAL record too large"))?;
        self.file.write_all(&length.to_le_bytes())?;
        self.file.write_all(&payload)?;
        self.sync()
    }
}