//! Files in the data directory (`/data` in the image; tests use temporary ones).

use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::keys::random_bytes;

#[derive(Debug, Clone)]
pub struct DataDir {
    root: PathBuf,
}

impl DataDir {
    pub fn new(root: impl Into<PathBuf>) -> DataDir {
        DataDir { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn database(&self) -> PathBuf {
        self.root.join("kohaku.db")
    }

    pub fn wal(&self) -> PathBuf {
        self.root.join("kohaku.db-wal")
    }

    pub fn shm(&self) -> PathBuf {
        self.root.join("kohaku.db-shm")
    }

    pub fn lock(&self) -> PathBuf {
        self.root.join("kohaku.lock")
    }

    pub fn backups(&self) -> PathBuf {
        self.root.join("backups")
    }

    /// `/data/backups`, created with mode 0700 when missing.
    pub fn ensure_backups(&self) -> io::Result<PathBuf> {
        let dir = self.backups();
        match DirBuilder::new().mode(0o700).create(&dir) {
            Ok(()) => sync_dir(&self.root)?,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        Ok(dir)
    }
}

/// A new, empty, mode-0600 `*.tmp` file in `dir`; retention removes one left behind.
pub fn create_tmp(dir: &Path, stem: &str) -> io::Result<(File, PathBuf)> {
    loop {
        let mut nonce = [0u8; 8];
        random_bytes(&mut nonce).map_err(io::Error::other)?;
        let hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let path = dir.join(format!(".{stem}.{hex}.tmp"));
        match OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((file, path)),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
}

/// Flushes a directory's entries (a new name, a removed one) to stable storage.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

/// The directory holding `path`, `.` for a bare file name.
pub fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

/// Removes a file Kohaku created, ignoring one already gone.
pub fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
