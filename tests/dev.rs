//! The `dev` build's differences, and nothing more (distribution: Development live
//! reload; change foundation D23). Compiled only with `--all-features`.

#![cfg(feature = "dev")]

mod support;

use axum::http::StatusCode;
use kohaku::assets;
use kohaku::dev::DEV_CSP;
use kohaku::http::headers::{CSP, RELEASE_CSP};
use kohaku::routing::hosts::{HostMap, ProjectId};
use support::*;

const PROJECT_HOST: &str = "bugs.example.net";

fn harness() -> Harness {
    let harness = Harness::new();
    harness
        .app
        .host_map
        .replace(HostMap::new(&harness.app.base_url).with_project(PROJECT_HOST, ProjectId(1)));
    harness
}

#[test]
fn only_connect_src_differs_from_release() {
    assert_eq!(DEV_CSP, format!("{RELEASE_CSP}; connect-src 'self'"));
    assert_eq!(CSP, DEV_CSP);
}

#[tokio::test]
async fn pages_carry_the_dev_policy_and_load_the_reload_script() {
    let harness = harness();
    let response = harness.get(MAIN_HOST, "/").await;
    assert_eq!(
        header_values(&response, "content-security-policy"),
        [DEV_CSP]
    );
    assert_eq!(header_values(&response, "cache-control"), ["no-cache"]);
    let html = body_text(response).await;
    assert!(
        html.contains(r#"<script src="/static/dev-reload.js" defer></script>"#),
        "{html}"
    );
    assert!(!html.contains("<script>"));
}

#[tokio::test]
async fn boot_id_and_reload_script_on_both_hosts() {
    let harness = harness();
    let mut ids = Vec::new();
    for host in [MAIN_HOST, PROJECT_HOST] {
        for _ in 0..2 {
            let response = harness.get(host, "/dev/boot-id").await;
            assert_eq!(response.status(), StatusCode::OK);
            ids.push(body_text(response).await);
        }
        let script = harness.get(host, "/static/dev-reload.js").await;
        assert_eq!(script.status(), StatusCode::OK);
        assert_eq!(header_values(&script, "content-type"), ["text/javascript"]);
        let text = body_text(script).await;
        assert!(text.contains("/dev/boot-id") && text.contains("location.reload()"));
    }
    assert_eq!(ids[0].len(), 32);
    assert!(ids.iter().all(|id| *id == ids[0]), "one id per process");
}

/// Restores the stylesheet even when the test fails.
struct Restore(std::path::PathBuf, Vec<u8>);

impl Drop for Restore {
    fn drop(&mut self) {
        std::fs::write(&self.0, &self.1).unwrap();
    }
}

#[tokio::test]
async fn an_edited_stylesheet_is_served_unbuilt() {
    let harness = harness();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static/kohaku.css");
    let original = std::fs::read(&path).unwrap();
    let _restore = Restore(path.clone(), original.clone());
    let mut edited = original.clone();
    edited.extend_from_slice(b"\n/* edited while running */\n");
    std::fs::write(&path, &edited).unwrap();
    let body = body_text(harness.get(MAIN_HOST, assets::path("kohaku.css")).await).await;
    assert_eq!(body.as_bytes(), &edited[..]);
}
