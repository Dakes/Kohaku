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

/// Every occurrence of `needle` in `src/`, as `file:line`, outside `allowed` files.
fn lines_with(needle: &str, allowed: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for file in rust_files(Path::new("src")) {
        let name = file.to_string_lossy().into_owned();
        if allowed.contains(&name.as_str()) {
            continue;
        }
        let text = fs::read_to_string(root().join(&file)).expect("read source file");
        for (number, line) in text.lines().enumerate() {
            if line.contains(needle) {
                found.push(format!("{name}:{}", number + 1));
            }
        }
    }
    found
}

#[test]
fn smtp_is_never_unencrypted() {
    // Clippy cannot ban enum variants.
    for variant in ["Tls::None", "Tls::Opportunistic"] {
        let found = lines_with(variant, &[]);
        assert!(found.is_empty(), "{variant} in {found:?}");
    }
}

#[test]
fn smtp_trusts_only_the_compiled_roots() {
    // `src/mail/smtp.rs` defines SmtpRoots; nothing in src/ may call for_tests or add
    // trust anchors of its own.
    let found = lines_with("SmtpRoots::for_tests", &[]);
    assert!(found.is_empty(), "SmtpRoots::for_tests in {found:?}");
    for anchor in [
        "add_root_certificate",
        "Certificate::from_pem",
        "Certificate::from_der",
        "CertificateStore::Default",
    ] {
        let found = lines_with(anchor, &["src/mail/smtp.rs"]);
        assert!(found.is_empty(), "{anchor} in {found:?}");
    }
}

#[test]
fn only_dev_builds_print_mail() {
    // The printer lives in src/mail/dev.rs, compiled only with `dev`; its one user
    // selects it under cfg(feature = "dev").
    let found = lines_with("PrintMailer", &["src/mail/dev.rs"]);
    for place in &found {
        let (file, line) = place.split_once(':').unwrap();
        let text = fs::read_to_string(root().join(file)).unwrap();
        let line: usize = line.parse().unwrap();
        let before: Vec<&str> = text.lines().take(line - 1).collect();
        let guarded = before
            .iter()
            .rev()
            .take(3)
            .any(|l| l.contains("cfg(feature = \"dev\")"));
        assert!(
            guarded,
            "PrintMailer outside cfg(feature = \"dev\") at {place}"
        );
    }
    let module = fs::read_to_string(root().join("src/mail/mod.rs")).unwrap();
    assert!(module.contains("#[cfg(feature = \"dev\")]\npub mod dev;"));
}

/// Every template file, with its text.
fn templates() -> Vec<(String, String)> {
    let dir = root().join("templates");
    let mut files: Vec<(String, String)> = fs::read_dir(&dir)
        .expect("read templates")
        .map(|entry| {
            let path = entry.expect("template entry").path();
            let text = fs::read_to_string(&path).expect("read template");
            (path.display().to_string(), text)
        })
        .collect();
    files.sort();
    files
}

/// Ways a template or template struct could turn off askama's escaping.
fn escaping_disabled(text: &str) -> Option<&'static str> {
    let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    [
        "|safe",
        "|escape(\"none\")",
        "|e(\"none\")",
        "{%autoescapefalse",
        "escape=\"none\"",
        "HtmlSafe",
    ]
    .into_iter()
    .find(|needle| squeezed.contains(needle))
}

#[test]
fn templates_never_disable_escaping() {
    for (name, text) in templates() {
        assert_eq!(escaping_disabled(&text), None, "{name}");
    }
    for file in rust_files(Path::new("src")) {
        let text = fs::read_to_string(root().join(&file)).expect("read source file");
        assert_eq!(escaping_disabled(&text), None, "{}", file.display());
    }
}

#[test]
fn the_escaping_check_catches_a_planted_safe() {
    for planted in [
        "{{ title|safe }}",
        "{{ title | safe }}",
        "{% autoescape false %}",
        "#[template(path = \"x\", escape = \"none\")]",
    ] {
        assert!(escaping_disabled(planted).is_some(), "{planted}");
    }
    assert_eq!(escaping_disabled("{{ title }}"), None);
}
