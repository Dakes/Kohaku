//! Source rules clippy cannot express (change foundation D3).

use std::fs;
use std::path::{Path, PathBuf};

/// The only files that may exempt themselves from `clippy.toml`'s `disallowed-methods`.
const EXEMPTION_ALLOWED: &[&str] = &["src/config.rs", "src/routing/build.rs"];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `dir`, as paths relative to the crate root.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root().join(dir)];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).expect("read source directory") {
            let path = entry.expect("read directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path.strip_prefix(root()).expect("under root").to_path_buf());
            }
        }
    }
    files.sort();
    files
}

/// Files under `src/` containing `needle`, excluding `allowed`.
fn offenders(needle: &str, allowed: &[&str]) -> Vec<String> {
    rust_files(Path::new("src"))
        .into_iter()
        .filter(|file| {
            let text = fs::read_to_string(root().join(file)).expect("read source file");
            text.contains(needle)
        })
        .map(|file| file.to_string_lossy().into_owned())
        .filter(|file| !allowed.contains(&file.as_str()))
        .collect()
}

#[test]
fn disallowed_method_exemptions_only_where_allowed() {
    let found = offenders("disallowed_methods", EXEMPTION_ALLOWED);
    assert!(
        found.is_empty(),
        "clippy.toml exemption outside {EXEMPTION_ALLOWED:?}: {found:?}"
    );
}
