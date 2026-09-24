//! Project administration, custom domains and the canonical redirect (projects; change
//! projects).

mod support;

use axum::body::Body;
use axum::http::StatusCode;
use kohaku::routing::hosts::{HostEntry, ProjectId};
use support::*;

/// An admin signed in, with TOTP as the admin role requires.
async fn admin(harness: &Harness) -> Browser {
    let account = harness.account("admin@example.org", "admin", true).await;
    harness.sign_in(&account).await
}

/// `(action, target_type, target_id, actor_label)` of every audit entry, oldest first.
async fn audit(harness: &Harness) -> Vec<(String, String, Option<i64>, String)> {
    harness
        .app
        .db
        .read(|conn| {
            conn.prepare(
                "SELECT action, target_type, target_id, actor_label FROM audit_log ORDER BY id",
            )?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
        .unwrap()
}

/// Every stored project column but `created_at`, by id.
async fn projects(harness: &Harness) -> Vec<Vec<rusqlite::types::Value>> {
    harness
        .app
        .db
        .read(|conn| {
            conn.prepare(
                "SELECT id, slug, name, public_host, screenshots_enabled, features_enabled,
                     require_email, plus_one_enabled, privacy_notice, security_contact,
                     next_number
                 FROM projects ORDER BY id",
            )?
            .query_map([], |r| {
                (0..11)
                    .map(|i| r.get::<_, rusqlite::types::Value>(i))
                    .collect()
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
        .unwrap()
}

async fn create(
    harness: &Harness,
    browser: &Browser,
    slug: &str,
    name: &str,
    host: &str,
) -> axum::http::Response<Body> {
    harness
        .post_as(
            browser,
            "/admin/projects",
            &[("slug", slug), ("name", name), ("public_host", host)],
        )
        .await
}

/// The settings form of a project, every field given.
fn settings<'a>(name: &'a str, host: &'a str) -> Vec<(&'a str, &'a str)> {
    vec![
        ("name", name),
        ("public_host", host),
        ("plus_one_enabled", "on"),
    ]
}

async fn tls_ask(harness: &Harness, domain: &str) -> StatusCode {
    harness
        .get(
            "kohaku:8080",
            &format!("/.well-known/kohaku/tls-ask?domain={domain}"),
        )
        .await
        .status()
}

#[tokio::test]
async fn defaults_of_a_new_project() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    let response = create(&harness, &browser, "demo", "Demo app", "").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header_values(&response, "location"),
        ["/admin/p/demo/settings"]
    );
    use rusqlite::types::Value::{Integer, Null, Text};
    assert_eq!(
        projects(&harness).await,
        [vec![
            Integer(1),
            Text("demo".into()),
            Text("Demo app".into()),
            Null,
            Integer(0),
            Integer(0),
            Integer(0),
            Integer(1),
            Null,
            Null,
            Integer(1),
        ]]
    );
    let entries = audit(&harness).await;
    assert_eq!(
        entries.last().unwrap(),
        &(
            "project.create".to_owned(),
            "project".to_owned(),
            Some(1),
            "admin".to_owned()
        )
    );
    // The settings page shows the defaults: only +1 is checked.
    let page = body_text(harness.get_as(&browser, "/admin/p/demo/settings").await).await;
    assert_eq!(page.matches(" checked").count(), 1);
    assert!(page.contains(r#"name="plus_one_enabled" checked"#));
}

#[tokio::test]
async fn attacker_controlled_text_cannot_hide_in_a_project_name() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    let long = "a".repeat(81);
    let long_slug = "a".repeat(41);
    for (slug, name, field) in [
        ("demo", "Demo\u{202E}ppa", "name"),
        ("demo", "Demo\u{200B}app", "name"),
        ("demo", "Demo\tapp", "name"),
        ("demo", "Demo\napp", "name"),
        ("demo", long.as_str(), "name"),
        ("Demo", "Demo app", "slug"),
        ("-demo", "Demo app", "slug"),
        (long_slug.as_str(), "Demo app", "slug"),
    ] {
        let response = create(&harness, &browser, slug, name, "").await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{slug} {name:?}"
        );
        assert_eq!(
            header_values(&response, "content-type"),
            ["text/html; charset=utf-8"]
        );
        let page = body_text(response).await;
        assert!(
            page.contains(&format!("The {field} must")),
            "{slug} {name:?}"
        );
        assert!(page.contains(r#"action="/admin/projects""#));
    }
    assert!(projects(&harness).await.is_empty());

    // The same rules on an existing project change nothing.
    create(&harness, &browser, "demo", "Demo app", "").await;
    let before = (projects(&harness).await, audit(&harness).await);
    for name in [
        "Demo\u{202E}ppa",
        "Demo\u{200B}app",
        "Demo\tapp",
        "Demo\napp",
        &long,
    ] {
        let response = harness
            .post_as(&browser, "/admin/p/demo/settings", &settings(name, ""))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{name:?}"
        );
        assert!(body_text(response).await.contains("The name must"));
    }
    for (field, value) in [
        ("privacy_notice", "a\u{202E}b".to_owned()),
        ("privacy_notice", "a".repeat(1025)),
        ("security_contact", "a\nb".to_owned()),
        ("security_contact", "a".repeat(255)),
    ] {
        let mut fields = settings("Demo app", "");
        fields.push((field, &value));
        let response = harness
            .post_as(&browser, "/admin/p/demo/settings", &fields)
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{field}"
        );
        let label = field.replace('_', " ");
        assert!(
            body_text(response)
                .await
                .contains(&format!("The {label} must"))
        );
    }
    assert_eq!((projects(&harness).await, audit(&harness).await), before);
}

#[tokio::test]
async fn settings_are_saved_audited_and_escaped() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    create(&harness, &browser, "demo", "Demo app", "").await;
    let entries = audit(&harness).await.len();
    let notice = "Line one\r\n<b>two</b>";
    let fields = [
        ("name", "  <script>x</script> "),
        ("public_host", ""),
        ("screenshots_enabled", "on"),
        ("features_enabled", "on"),
        ("require_email", "on"),
        ("privacy_notice", notice),
        ("security_contact", "security@example.net"),
    ];
    let response = harness
        .post_as(&browser, "/admin/p/demo/settings", &fields)
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    use rusqlite::types::Value::{Integer, Null, Text};
    assert_eq!(
        projects(&harness).await[0][2..10],
        [
            Text("<script>x</script>".into()),
            Null,
            Integer(1),
            Integer(1),
            Integer(1),
            Integer(0),
            Text("Line one\n<b>two</b>".into()),
            Text("security@example.net".into()),
        ]
    );
    let log = audit(&harness).await;
    assert_eq!(log.len(), entries + 1);
    assert_eq!(log.last().unwrap().0, "project.update");
    assert_eq!(log.last().unwrap().2, Some(1));

    let page = body_text(harness.get_as(&browser, "/admin/p/demo/settings").await).await;
    assert!(!page.contains("<script>x") && !page.contains("<b>two"));
    assert!(page.contains("&#60;script&#62;x&#60;/script&#62;"));

    // Saving the same values again changes nothing and records nothing.
    let response = harness
        .post_as(&browser, "/admin/p/demo/settings", &fields)
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(audit(&harness).await.len(), entries + 1);
}

#[tokio::test]
async fn invalid_or_taken_hosts() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    create(&harness, &browser, "other", "Other", "bugs.example.net").await;
    create(&harness, &browser, "demo", "Demo app", "").await;
    let before = (projects(&harness).await, audit(&harness).await);
    for host in [
        "192.0.2.1",
        "localhost",
        MAIN_HOST,
        "Kohaku.Example.ORG",
        "bugs.example.net.",
        "bugs.example.net",
        " BUGS.example.NET ",
    ] {
        let response = harness
            .post_as(
                &browser,
                "/admin/p/demo/settings",
                &settings("Demo app", host),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{host}"
        );
        assert!(
            body_text(response).await.contains("The custom domain"),
            "{host}"
        );
        let response = create(&harness, &browser, "fresh", "Fresh", host).await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{host}"
        );
    }
    assert_eq!((projects(&harness).await, audit(&harness).await), before);
    // A taken slug is refused the same way.
    let response = create(&harness, &browser, "demo", "Again", "").await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(body_text(response).await.contains("The slug is taken"));
    assert_eq!((projects(&harness).await, audit(&harness).await), before);
}

#[tokio::test]
async fn attacker_claims_someone_elses_domain() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    assert_eq!(
        tls_ask(&harness, "evil.example").await,
        StatusCode::NOT_FOUND
    );
    create(&harness, &browser, "a", "Project A", " Bugs.Example.NET ").await;
    assert_eq!(tls_ask(&harness, "bugs.example.net").await, StatusCode::OK);
    assert_eq!(
        tls_ask(&harness, "evil.example").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        harness
            .get("bugs.example.net", "/does-not-exist")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    let response = harness
        .post_as(
            &browser,
            "/admin/p/a/settings",
            &settings("Project A", "bugs.example.org"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    // The move is live when the request returns.
    assert_eq!(
        tls_ask(&harness, "bugs.example.net").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(tls_ask(&harness, "bugs.example.org").await, StatusCode::OK);
    assert_eq!(
        harness.get("bugs.example.net", "/").await.status(),
        StatusCode::MISDIRECTED_REQUEST
    );
    assert_eq!(
        harness.app.host_map.current().get("bugs.example.org"),
        Some(HostEntry::Project(ProjectId(1)))
    );

    // Removing the domain removes the host.
    harness
        .post_as(&browser, "/admin/p/a/settings", &settings("Project A", ""))
        .await;
    assert_eq!(
        tls_ask(&harness, "bugs.example.org").await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn maintainer_probes_the_project_settings() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    create(&harness, &browser, "demo", "Demo app", "bugs.example.net").await;
    let maintainer = harness.account("m@example.org", "maintainer", false).await;
    let maintainer = harness.sign_in(&maintainer).await;
    let before = (projects(&harness).await, audit(&harness).await);
    let missing = harness.get_as(&maintainer, "/admin/does-not-exist").await;
    let missing = body_text(missing).await;
    for path in ["/admin/projects", "/admin/p/demo/settings"] {
        let response = harness.get_as(&maintainer, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body_text(response).await, missing, "{path}");
    }
    for (path, fields) in [
        ("/admin/p/demo/delete", vec![("confirm", "demo")]),
        ("/admin/p/demo/settings", settings("Mine", "")),
        (
            "/admin/projects",
            vec![("slug", "mine"), ("name", "Mine"), ("public_host", "")],
        ),
    ] {
        let response = harness.post_as(&maintainer, path, &fields).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!((projects(&harness).await, audit(&harness).await), before);
    // Neither the navigation nor the home page shows projects to a maintainer.
    let home = body_text(harness.get_as(&maintainer, "/admin").await).await;
    assert!(!home.contains("/admin/projects") && !home.contains("Demo app"));
}

#[tokio::test]
async fn admin_home_lists_every_project() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    let home = body_text(harness.get_as(&browser, "/admin").await).await;
    assert!(home.contains("No projects yet"));
    create(&harness, &browser, "zeta", "Zeta & co", "").await;
    create(&harness, &browser, "alpha", "Alpha", "bugs.example.net").await;
    let home = body_text(harness.get_as(&browser, "/admin").await).await;
    let (alpha, zeta) = (
        home.find("Alpha").unwrap(),
        home.find("Zeta &#38; co").unwrap(),
    );
    assert!(alpha < zeta, "listed by slug");
    assert!(home.contains("bugs.example.net"));
    let list = body_text(harness.get_as(&browser, "/admin/projects").await).await;
    assert!(list.contains(r#"href="/admin/p/alpha/settings""#));
    assert!(list.contains(r#"href="/admin/p/zeta/settings""#));
}

#[tokio::test]
async fn deletion_needs_the_slug() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    create(&harness, &browser, "demo", "Demo app", "bugs.example.net").await;
    let before = (projects(&harness).await, audit(&harness).await);
    for confirm in ["dem", "", "DEMO", " demo"] {
        let response = harness
            .post_as(&browser, "/admin/p/demo/delete", &[("confirm", confirm)])
            .await;
        assert_eq!(
            response.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{confirm:?}"
        );
        assert!(body_text(response).await.contains("type its slug exactly"));
    }
    let response = harness.post_as(&browser, "/admin/p/demo/delete", &[]).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!((projects(&harness).await, audit(&harness).await), before);
    assert_eq!(tls_ask(&harness, "bugs.example.net").await, StatusCode::OK);

    let response = harness
        .post_as(&browser, "/admin/p/demo/delete", &[("confirm", "demo")])
        .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(header_values(&response, "location"), ["/admin/projects"]);
    assert!(projects(&harness).await.is_empty());
    assert_eq!(
        audit(&harness).await.last().unwrap(),
        &(
            "project.delete".to_owned(),
            "project".to_owned(),
            Some(1),
            "admin".to_owned()
        )
    );
    // The host leaves the map at once.
    assert_eq!(
        tls_ask(&harness, "bugs.example.net").await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        harness.get("bugs.example.net", "/").await.status(),
        StatusCode::MISDIRECTED_REQUEST
    );
    assert_eq!(
        harness.get(MAIN_HOST, "/p/demo").await.status(),
        StatusCode::NOT_FOUND
    );
    // Its pages are gone, and a new project never gets its id.
    assert_eq!(
        harness
            .get_as(&browser, "/admin/p/demo/settings")
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    create(&harness, &browser, "demo", "Demo app", "").await;
    assert_eq!(
        projects(&harness).await[0][0],
        rusqlite::types::Value::Integer(2)
    );
}

#[tokio::test]
async fn unknown_or_malformed_slugs_get_404() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    let missing = body_text(harness.get_as(&browser, "/admin/does-not-exist").await).await;
    for path in [
        "/admin/p/nothing/settings",
        "/admin/p/Nothing/settings",
        "/admin/p/%2e%2e/settings",
        "/admin/p/a'b/settings",
    ] {
        let response = harness.get_as(&browser, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(body_text(response).await, missing, "{path}");
        let path = path.replace("/settings", "/delete");
        let response = harness
            .post_as(&browser, &path, &[("confirm", "nothing")])
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn old_links_follow_the_domain() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    create(&harness, &browser, "demo", "Demo app", "bugs.example.net").await;
    for (method, path, location) in [
        (
            "GET",
            "/p/demo/r/12?x=1",
            "https://bugs.example.net/r/12?x=1",
        ),
        ("HEAD", "/p/demo", "https://bugs.example.net/"),
        ("GET", "/p/demo/", "https://bugs.example.net/"),
        (
            "GET",
            "/p/demo?a=%3Cb%3E",
            "https://bugs.example.net/?a=%3Cb%3E",
        ),
        (
            "GET",
            "/p/demo/r/%2F%2Fevil",
            "https://bugs.example.net/r/%2F%2Fevil",
        ),
        (
            "GET",
            "/p/demo//evil.example",
            "https://bugs.example.net//evil.example",
        ),
        ("GET", "/p/demo/apix", "https://bugs.example.net/apix"),
    ] {
        let response = harness
            .send(
                request(method, MAIN_HOST, path)
                    .header("x-forwarded-host", "evil.example")
                    .header("x-forwarded-proto", "http")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::PERMANENT_REDIRECT, "{path}");
        assert_eq!(header_values(&response, "location"), [location], "{path}");
        assert_eq!(header_values(&response, "cache-control"), ["max-age=3600"]);
        assert!(header_values(&response, "set-cookie").is_empty());
        assert!(body_text(response).await.is_empty(), "{path}");
    }
}

#[tokio::test]
async fn api_and_writes_are_never_redirected() {
    let harness = Harness::new();
    let browser = admin(&harness).await;
    create(&harness, &browser, "demo", "Demo app", "bugs.example.net").await;
    for path in ["/p/demo/api/v1/reports", "/p/demo/api", "/p/demo/api/"] {
        let response = harness.get(MAIN_HOST, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(header_values(&response, "cache-control"), ["no-store"]);
    }
    for method in ["POST", "PUT", "DELETE"] {
        let response = harness
            .send(
                request(method, MAIN_HOST, "/p/demo/new")
                    .header("origin", "https://kohaku.example.org")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .body(Body::from("title=x"))
                    .unwrap(),
            )
            .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method}");
        assert!(header_values(&response, "location").is_empty());
    }
}

#[tokio::test]
async fn watcher_picks_up_changes_by_another_process_and_survives_failures() {
    let harness = Harness::new();
    let (stop, stopped) = tokio::sync::watch::channel(());
    let watcher = tokio::spawn(kohaku::routing::hosts::watch(
        std::sync::Arc::clone(&harness.app),
        stopped,
    ));
    let other = kohaku::db::open(&harness.data.database(), kohaku::db::Role::Command).unwrap();
    let seen = |host: &'static str| {
        let app = std::sync::Arc::clone(&harness.app);
        async move {
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(2500);
            while tokio::time::Instant::now() < deadline {
                if app.host_map.current().get(host).is_some() {
                    return true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            false
        }
    };
    other
        .execute(
            "INSERT INTO projects (slug, name, public_host, created_at)
             VALUES ('demo', 'Demo', 'bugs.example.net', 1)",
            [],
        )
        .unwrap();
    assert!(seen("bugs.example.net").await, "within 2 seconds");

    // A failing rebuild keeps the old map; the next good one replaces it.
    let logs = LogCapture::default();
    let guard = logs.install();
    other
        .execute_batch("ALTER TABLE projects RENAME TO projects_away")
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(4500)).await;
    assert!(
        harness
            .app
            .host_map
            .current()
            .get("bugs.example.net")
            .is_some()
    );
    assert_eq!(
        logs.text().matches("cannot rebuild the host map").count(),
        1
    );
    drop(guard);
    other
        .execute_batch(
            "ALTER TABLE projects_away RENAME TO projects;
             UPDATE projects SET public_host = 'bugs.example.org';",
        )
        .unwrap();
    assert!(seen("bugs.example.org").await);
    assert!(
        harness
            .app
            .host_map
            .current()
            .get("bugs.example.net")
            .is_none()
    );
    stop.send(()).unwrap();
    watcher.await.unwrap();
}
