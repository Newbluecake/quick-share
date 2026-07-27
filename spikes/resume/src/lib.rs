//! Disposable crash-consistency experiment for resumable chunk writes.
//! It intentionally implements only one file and is not production storage code.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResumeError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid journal: {0}")]
    InvalidJournal(#[from] serde_json::Error),
    #[error("chunk {index} is outside the file")]
    InvalidChunk { index: usize },
    #[error("chunk {index} has length {actual}, expected {expected}")]
    InvalidLength {
        index: usize,
        actual: usize,
        expected: usize,
    },
    #[error("chunk {index} digest mismatch")]
    DigestMismatch { index: usize },
    #[error("chunk {index} was already recorded with a different digest")]
    ConflictingChunk { index: usize },
    #[error("not every chunk has been recorded")]
    Incomplete,
    #[error("final digest mismatch")]
    FinalDigestMismatch,
    #[error("injected crash at {0:?}")]
    InjectedCrash(CrashPoint),
    #[error("final target already exists")]
    FinalExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashPoint {
    None,
    AfterDataSync,
    BeforeJournalRename,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Journal {
    version: u8,
    file_size: u64,
    chunk_size: u32,
    completed: Vec<Option<String>>,
}

pub struct ResumeReceiver {
    root: PathBuf,
    part_path: PathBuf,
    state_path: PathBuf,
    final_path: PathBuf,
    journal: Journal,
}

impl ResumeReceiver {
    pub fn create(
        root: impl AsRef<Path>,
        final_name: &str,
        file_size: u64,
        chunk_size: u32,
    ) -> Result<Self, ResumeError> {
        if chunk_size == 0 {
            return Err(ResumeError::InvalidChunk { index: 0 });
        }
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        let chunk_count = file_size.div_ceil(u64::from(chunk_size)) as usize;
        let journal = Journal {
            version: 1,
            file_size,
            chunk_size,
            completed: vec![None; chunk_count],
        };
        let receiver = Self {
            part_path: root.join("payload.part"),
            state_path: root.join("state.json"),
            final_path: root.join(final_name),
            root,
            journal,
        };
        let file = File::create(&receiver.part_path)?;
        file.set_len(file_size)?;
        file.sync_all()?;
        receiver.save_journal(CrashPoint::None)?;
        Ok(receiver)
    }

    pub fn reopen(root: impl AsRef<Path>, final_name: &str) -> Result<Self, ResumeError> {
        let root = root.as_ref().to_path_buf();
        let bytes = fs::read(root.join("state.json"))?;
        let journal: Journal = serde_json::from_slice(&bytes)?;
        if journal.version != 1 || journal.chunk_size == 0 {
            return Err(ResumeError::InvalidChunk { index: 0 });
        }
        let expected_count = journal.file_size.div_ceil(u64::from(journal.chunk_size)) as usize;
        if journal.completed.len() != expected_count {
            return Err(ResumeError::InvalidChunk {
                index: journal.completed.len(),
            });
        }
        let part_path = root.join("payload.part");
        if fs::metadata(&part_path)?.len() != journal.file_size {
            return Err(ResumeError::InvalidLength {
                index: 0,
                actual: fs::metadata(&part_path)?.len() as usize,
                expected: journal.file_size as usize,
            });
        }
        Ok(Self {
            part_path,
            state_path: root.join("state.json"),
            final_path: root.join(final_name),
            root,
            journal,
        })
    }

    pub fn missing_chunks(&self) -> Vec<usize> {
        self.journal
            .completed
            .iter()
            .enumerate()
            .filter_map(|(index, digest)| digest.is_none().then_some(index))
            .collect()
    }

    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    pub fn write_chunk(
        &mut self,
        index: usize,
        bytes: &[u8],
        expected_digest: &str,
        crash: CrashPoint,
    ) -> Result<(), ResumeError> {
        let expected_len = self.expected_chunk_len(index)?;
        if bytes.len() != expected_len {
            return Err(ResumeError::InvalidLength {
                index,
                actual: bytes.len(),
                expected: expected_len,
            });
        }
        let actual_digest = blake3::hash(bytes).to_hex().to_string();
        if actual_digest != expected_digest {
            return Err(ResumeError::DigestMismatch { index });
        }
        if let Some(recorded) = &self.journal.completed[index] {
            return if recorded == expected_digest {
                Ok(())
            } else {
                Err(ResumeError::ConflictingChunk { index })
            };
        }

        let mut part = OpenOptions::new().write(true).open(&self.part_path)?;
        let offset = index as u64 * u64::from(self.journal.chunk_size);
        part.seek(SeekFrom::Start(offset))?;
        part.write_all(bytes)?;
        part.sync_data()?;
        if crash == CrashPoint::AfterDataSync {
            return Err(ResumeError::InjectedCrash(crash));
        }

        self.journal.completed[index] = Some(actual_digest);
        if let Err(error) = self.save_journal(crash) {
            self.journal.completed[index] = None;
            return Err(error);
        }
        Ok(())
    }

    pub fn finalize(mut self, expected_digest: &str) -> Result<PathBuf, ResumeError> {
        if !self.missing_chunks().is_empty() {
            return Err(ResumeError::Incomplete);
        }
        if self.final_path.exists() {
            return Err(ResumeError::FinalExists);
        }
        let actual = hash_file(&self.part_path)?;
        if actual != expected_digest {
            return Err(ResumeError::FinalDigestMismatch);
        }

        // Persist a completed marker before rename is deliberately omitted here:
        // after rename, the final digest is sufficient evidence. Production code
        // needs a richer reconciliation state machine around this boundary.
        fs::rename(&self.part_path, &self.final_path)?;
        sync_directory(&self.root)?;
        let _ = fs::remove_file(&self.state_path);
        Ok(std::mem::take(&mut self.final_path))
    }

    fn expected_chunk_len(&self, index: usize) -> Result<usize, ResumeError> {
        if index >= self.journal.completed.len() {
            return Err(ResumeError::InvalidChunk { index });
        }
        let start = index as u64 * u64::from(self.journal.chunk_size);
        let remaining = self.journal.file_size - start;
        Ok(remaining.min(u64::from(self.journal.chunk_size)) as usize)
    }

    fn save_journal(&self, crash: CrashPoint) -> Result<(), ResumeError> {
        let mut temp = NamedTempFile::new_in(&self.root)?;
        serde_json::to_writer_pretty(&mut temp, &self.journal)?;
        temp.write_all(b"\n")?;
        temp.as_file_mut().sync_all()?;
        if crash == CrashPoint::BeforeJournalRename {
            return Err(ResumeError::InjectedCrash(crash));
        }
        temp.persist(&self.state_path)
            .map_err(|error| ResumeError::Io(error.error))?;
        sync_directory(&self.root)?;
        Ok(())
    }
}

pub fn hash_file(path: impl AsRef<Path>) -> Result<String, ResumeError> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 256 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), ResumeError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), ResumeError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const CHUNK_SIZE: u32 = 4;

    fn digest(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    #[test]
    fn final_path_is_hidden_until_every_chunk_is_verified() {
        // Arrange
        let temp = tempdir().expect("tempdir");
        let mut receiver =
            ResumeReceiver::create(temp.path(), "final.bin", 8, CHUNK_SIZE).expect("create");

        // Act
        receiver
            .write_chunk(0, b"abcd", &digest(b"abcd"), CrashPoint::None)
            .expect("first chunk");

        // Assert
        assert!(!receiver.final_path().exists());
        assert_eq!(receiver.missing_chunks(), vec![1]);
    }

    #[test]
    fn crash_after_data_sync_is_recovered_by_rewriting_unrecorded_chunk() {
        // Arrange
        let temp = tempdir().expect("tempdir");
        let mut receiver =
            ResumeReceiver::create(temp.path(), "final.bin", 8, CHUNK_SIZE).expect("create");

        // Act
        let failure = receiver.write_chunk(0, b"abcd", &digest(b"abcd"), CrashPoint::AfterDataSync);
        drop(receiver);
        let mut reopened = ResumeReceiver::reopen(temp.path(), "final.bin").expect("reopen");

        // Assert
        assert!(matches!(failure, Err(ResumeError::InjectedCrash(_))));
        assert_eq!(reopened.missing_chunks(), vec![0, 1]);
        reopened
            .write_chunk(0, b"abcd", &digest(b"abcd"), CrashPoint::None)
            .expect("rewrite chunk");
    }

    #[test]
    fn crash_before_journal_rename_keeps_previous_valid_journal() {
        // Arrange
        let temp = tempdir().expect("tempdir");
        let mut receiver =
            ResumeReceiver::create(temp.path(), "final.bin", 8, CHUNK_SIZE).expect("create");

        // Act
        let failure = receiver.write_chunk(
            0,
            b"abcd",
            &digest(b"abcd"),
            CrashPoint::BeforeJournalRename,
        );
        drop(receiver);
        let reopened = ResumeReceiver::reopen(temp.path(), "final.bin").expect("reopen");

        // Assert
        assert!(matches!(failure, Err(ResumeError::InjectedCrash(_))));
        assert_eq!(reopened.missing_chunks(), vec![0, 1]);
    }

    #[test]
    fn out_of_order_chunks_resume_and_finalize_with_full_digest() {
        // Arrange
        let temp = tempdir().expect("tempdir");
        let payload = b"abcdefghij";
        let expected = digest(payload);
        let mut receiver =
            ResumeReceiver::create(temp.path(), "final.bin", 10, CHUNK_SIZE).expect("create");

        // Act
        receiver
            .write_chunk(2, b"ij", &digest(b"ij"), CrashPoint::None)
            .expect("last chunk");
        receiver
            .write_chunk(0, b"abcd", &digest(b"abcd"), CrashPoint::None)
            .expect("first chunk");
        drop(receiver);
        let mut reopened = ResumeReceiver::reopen(temp.path(), "final.bin").expect("reopen");
        reopened
            .write_chunk(1, b"efgh", &digest(b"efgh"), CrashPoint::None)
            .expect("middle chunk");
        let final_path = reopened.finalize(&expected).expect("finalize");

        // Assert
        assert_eq!(fs::read(final_path).expect("read final"), payload);
        assert!(!temp.path().join("payload.part").exists());
    }

    #[test]
    fn duplicate_identical_chunk_is_idempotent_but_wrong_digest_is_rejected() {
        // Arrange
        let temp = tempdir().expect("tempdir");
        let mut receiver =
            ResumeReceiver::create(temp.path(), "final.bin", 4, CHUNK_SIZE).expect("create");
        let correct = digest(b"abcd");
        receiver
            .write_chunk(0, b"abcd", &correct, CrashPoint::None)
            .expect("first write");

        // Act + Assert
        receiver
            .write_chunk(0, b"abcd", &correct, CrashPoint::None)
            .expect("idempotent duplicate");
        let wrong = receiver.write_chunk(0, b"wxyz", &digest(b"wxyz"), CrashPoint::None);
        assert!(matches!(
            wrong,
            Err(ResumeError::ConflictingChunk { index: 0 })
        ));
    }

    #[test]
    fn truncated_journal_fails_closed_without_exposing_final_file() {
        // Arrange
        let temp = tempdir().expect("tempdir");
        let receiver =
            ResumeReceiver::create(temp.path(), "final.bin", 4, CHUNK_SIZE).expect("create");
        drop(receiver);
        fs::write(temp.path().join("state.json"), b"{broken").expect("truncate journal");

        // Act
        let result = ResumeReceiver::reopen(temp.path(), "final.bin");

        // Assert
        assert!(matches!(result, Err(ResumeError::InvalidJournal(_))));
        assert!(!temp.path().join("final.bin").exists());
    }
}
