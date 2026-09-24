//! Test helpers shared by unit tests (`crate::test_support`) and integration tests
//! (`tests/support/`, which includes this file by path). Uses no crate paths, so it
//! compiles in both places.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::{self, DirBuilder};
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A valid `KOHAKU_SECRET`: base64 of the 32 bytes `kohaku-test-secret-32-bytes-long`.
pub const TEST_SECRET: &str = "a29oYWt1LXRlc3Qtc2VjcmV0LTMyLWJ5dGVzLWxvbmc=";

/// A second valid secret, for keycheck mismatches.
pub const OTHER_TEST_SECRET: &str = "b3RoZXIta29oYWt1LXRlc3Qtc2VjcmV0LTMyLWJ5dGU=";

/// The SMTP password in [`valid_environment`].
pub const TEST_SMTP_PASSWORD: &str = "test-smtp-password";

/// A complete, valid `kohaku serve` environment for this build: a `dev` build gets no
/// SMTP connection settings, which it refuses.
pub fn valid_environment() -> HashMap<String, OsString> {
    let mut vars = vec![
        ("KOHAKU_BASE_URL", "https://kohaku.example.org"),
        ("KOHAKU_TRUSTED_PROXIES", "none"),
        ("KOHAKU_SECRET", TEST_SECRET),
        ("KOHAKU_SMTP_FROM", "kohaku@kohaku.example.org"),
    ];
    if cfg!(not(feature = "dev")) {
        vars.extend([
            ("KOHAKU_SMTP_HOST", "smtp.kohaku.test"),
            ("KOHAKU_SMTP_PORT", "587"),
            ("KOHAKU_SMTP_TLS", "starttls"),
            ("KOHAKU_SMTP_USERNAME", "kohaku"),
            ("KOHAKU_SMTP_PASSWORD", TEST_SMTP_PASSWORD),
        ]);
    }
    vars.into_iter()
        .map(|(name, value)| (name.to_owned(), OsString::from(value)))
        .collect()
}

/// A new, empty, mode-0700 directory under the system temporary directory, removed with
/// everything in it when dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new() -> TempDir {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        loop {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("kohaku-test-{}-{nanos}-{n}", std::process::id()));
            // create() fails on any existing entry, so no directory is ever reused.
            match DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return TempDir { path },
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create a temporary directory: {error}"),
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        // A test may have made entries read-only; removal is best effort.
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn temp_dirs_are_unique_private_and_removed() {
        let dirs: Vec<TempDir> = (0..100).map(|_| TempDir::new()).collect();
        let paths: HashSet<PathBuf> = dirs.iter().map(|d| d.path().to_path_buf()).collect();
        assert_eq!(paths.len(), 100);
        for dir in &dirs {
            let meta = fs::metadata(dir.path()).unwrap();
            assert!(meta.is_dir());
            assert_eq!(meta.permissions().mode() & 0o777, 0o700);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
        }
        let kept = TempDir::new();
        fs::write(kept.path().join("f"), b"x").unwrap();
        let path = kept.path().to_path_buf();
        drop(kept);
        assert!(!path.exists());
        drop(dirs);
        assert!(paths.iter().all(|p| !p.exists()));
    }

    #[test]
    fn existing_entries_are_never_reused() {
        let first = TempDir::new();
        fs::write(first.path().join("marker"), b"keep").unwrap();
        let second = TempDir::new();
        assert_ne!(first.path(), second.path());
        assert_eq!(fs::read(first.path().join("marker")).unwrap(), b"keep");
        assert_eq!(fs::read_dir(second.path()).unwrap().count(), 0);
    }
}
