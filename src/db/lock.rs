//! The instance lock on `/data/kohaku.lock` (data-storage: Instance lock; D9).

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::os::unix::fs::OpenOptionsExt;

use super::paths::DataDir;

/// Held until dropped; the operating system releases it whenever the process ends.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

#[derive(Debug)]
pub enum LockError {
    Held,
    Io(io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Held => f.write_str(
                "another Kohaku process is using /data (kohaku serve or kohaku restore); stop it first",
            ),
            LockError::Io(error) => write!(f, "cannot lock /data/kohaku.lock ({})", error.kind()),
        }
    }
}

impl std::error::Error for LockError {}

impl InstanceLock {
    /// Takes the exclusive lock, creating `kohaku.lock` with mode 0600 if missing.
    /// When another process holds it, nothing is created or changed.
    pub fn acquire(data: &DataDir) -> Result<InstanceLock, LockError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(data.lock())
            .map_err(LockError::Io)?;
        match file.try_lock() {
            Ok(()) => Ok(InstanceLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(LockError::Held),
            Err(TryLockError::Error(error)) => Err(LockError::Io(error)),
        }
    }
}
