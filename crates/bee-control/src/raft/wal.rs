use std::fs::{File, OpenOptions};
use std::io::{Error, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::types::{LogEntry, NodeId, Term};

const MAGIC: &[u8; 8] = b"BEERAWL1";

#[derive(Debug, Serialize, Deserialize)]
enum WalRecord {
    Entry(LogEntry),
    TermAndVote {
        term: Term,
        voted_for: Option<NodeId>,
    },
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct WalReplay {
    pub entries: Vec<LogEntry>,
    pub term: Term,
    pub voted_for: Option<NodeId>,
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

    pub fn append(&mut self, entry: &LogEntry) -> std::io::Result<()> {
        self.write_record(&WalRecord::Entry(entry.clone()))
    }

    pub fn persist_term_and_vote(
        &mut self,
        term: Term,
        voted_for: Option<NodeId>,
    ) -> std::io::Result<()> {
        self.write_record(&WalRecord::TermAndVote { term, voted_for })
    }

    pub fn replay(path: &Path) -> std::io::Result<WalReplay> {
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
                WalRecord::Entry(entry) => replay.entries.push(entry),
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
