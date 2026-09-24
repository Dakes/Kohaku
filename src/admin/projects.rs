//! Project administration: the list and create form, the settings page and deletion,
//! for the admin only (projects: Project administration, Custom domains; change
//! projects D3).

use askama::Template;
use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Form};
use serde::Deserialize;

use super::Nav;
use crate::audit::Actor;
use crate::auth::session::{SessionUser, see_other};
use crate::db::migrate::DbFailure;
use crate::http::errors::OwnBody;
use crate::pages::{Chrome, html};
use crate::projects::{
    self, Problem, Project, ProjectError, Settings, parse_host, parse_name, parse_privacy_notice,
    parse_security_contact, parse_slug,
};
use crate::routing::AppState;
use crate::routing::hosts::refresh;

const LIST_PATH: &str = "/admin/projects";

/// Wrong or missing slug in the deletion form.
const CONFIRM_FAILED: &str =
    "The project was not deleted: type its slug exactly to confirm the deletion.";

/// The create form's values, as entered.
#[derive(Deserialize, Default)]
pub struct CreateForm {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    public_host: String,
}

/// The settings form's values, as entered; an unchecked box sends nothing.
#[derive(Deserialize)]
pub struct SettingsForm {
    #[serde(default)]
    name: String,
    #[serde(default)]
    public_host: String,
    screenshots_enabled: Option<String>,
    features_enabled: Option<String>,
    require_email: Option<String>,
    plus_one_enabled: Option<String>,
    #[serde(default)]
    privacy_notice: String,
    #[serde(default)]
    security_contact: String,
}

impl SettingsForm {
    fn of(settings: &Settings) -> SettingsForm {
        let on = |value: bool| value.then(|| "on".to_owned());
        SettingsForm {
            name: settings.name.clone(),
            public_host: settings.public_host.clone().unwrap_or_default(),
            screenshots_enabled: on(settings.screenshots_enabled),
            features_enabled: on(settings.features_enabled),
            require_email: on(settings.require_email),
            plus_one_enabled: on(settings.plus_one_enabled),
            privacy_notice: settings.privacy_notice.clone().unwrap_or_default(),
            security_contact: settings.security_contact.clone().unwrap_or_default(),
        }
    }

    fn parse(&self, app: &AppState) -> Result<Settings, Problem> {
        Ok(Settings {
            name: parse_name(&self.name)?,
            public_host: parse_host(&self.public_host, &app.base_url)?,
            screenshots_enabled: self.screenshots_enabled.is_some(),
            features_enabled: self.features_enabled.is_some(),
            require_email: self.require_email.is_some(),
            plus_one_enabled: self.plus_one_enabled.is_some(),
            privacy_notice: parse_privacy_notice(&self.privacy_notice)?,
            security_contact: parse_security_contact(&self.security_contact)?,
        })
    }
}

#[derive(Deserialize)]
pub struct DeleteForm {
    #[serde(default)]
    confirm: String,
}

#[derive(Template)]
#[template(path = "projects.html")]
struct ListPage {
    chrome: Chrome,
    nav: Nav,
    projects: Vec<Project>,
    form: CreateForm,
    error: Option<&'static str>,
}

#[derive(Template)]
#[template(path = "project_settings.html")]
struct SettingsPage {
    chrome: Chrome,
    nav: Nav,
    slug: String,
    form: SettingsForm,
    error: Option<&'static str>,
    delete_error: Option<&'static str>,
}

fn server_error(failure: &DbFailure) -> Response {
    tracing::error!("project administration failed: {failure}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// A refused form shown again with its problem: 422, the page as the body.
fn refused(template: &impl Template) -> Response {
    let mut response = html(template);
    if response.status() == StatusCode::OK {
        *response.status_mut() = StatusCode::UNPROCESSABLE_ENTITY;
        response.extensions_mut().insert(OwnBody);
    }
    response
}

/// `303` to a project's settings page; slugs are URL-safe by their rule.
fn to_settings(slug: &str) -> Response {
    match HeaderValue::try_from(format!("/admin/p/{slug}/settings")) {
        Ok(location) => (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn actor(user: &SessionUser) -> Actor {
    Actor::User {
        id: user.user_id,
        role: user.role,
    }
}

/// Publishes a committed change to the host map at once. On failure the watcher still
/// picks it up within 2 seconds, so the admin's request succeeds.
async fn publish(app: &AppState) {
    if let Err(failure) = refresh(app).await {
        tracing::error!("cannot rebuild the host map after a project change: {failure}");
    }
}

/// The project named by a path's slug; `None` for a malformed or unknown one.
async fn find(app: &AppState, slug: &str) -> Result<Option<Project>, DbFailure> {
    let Ok(slug) = parse_slug(slug) else {
        return Ok(None);
    };
    app.db
        .read(move |conn| projects::find(conn, &slug).map_err(DbFailure::from))
        .await
}

async fn list_page(
    app: &AppState,
    user: &SessionUser,
    form: CreateForm,
    error: Option<&'static str>,
) -> Response {
    let projects = app
        .db
        .read(|conn| projects::list(conn).map_err(DbFailure::from))
        .await;
    let page = match projects {
        Ok(projects) => ListPage {
            chrome: Chrome::new(),
            nav: Nav::of(user),
            projects,
            form,
            error,
        },
        Err(failure) => return server_error(&failure),
    };
    if error.is_some() {
        refused(&page)
    } else {
        html(&page)
    }
}

/// `GET /admin/projects`.
pub async fn list(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
) -> Response {
    list_page(&app, &user, CreateForm::default(), None).await
}

/// `POST /admin/projects`: a new project with the defaults, then its settings page.
pub async fn create(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
    Form(form): Form<CreateForm>,
) -> Response {
    let parsed = parse_slug(&form.slug).and_then(|slug| {
        let name = parse_name(&form.name)?;
        let host = parse_host(&form.public_host, &app.base_url)?;
        Ok((slug, Settings::new(name, host)))
    });
    let (slug, settings) = match parsed {
        Ok(parsed) => parsed,
        Err(problem) => return list_page(&app, &user, form, Some(problem.message)).await,
    };
    let (actor, now, new_slug) = (actor(&user), app.clock.unix(), slug.clone());
    let created = app
        .db
        .write(move |tx| projects::create(tx, &new_slug, &settings, actor, now))
        .await;
    match created {
        Ok(_) => {
            publish(&app).await;
            to_settings(&slug)
        }
        Err(ProjectError::Invalid(problem)) => {
            list_page(&app, &user, form, Some(problem.message)).await
        }
        Err(ProjectError::Database(failure)) => server_error(&failure),
    }
}

fn settings_page(
    user: &SessionUser,
    slug: String,
    form: SettingsForm,
    error: Option<&'static str>,
    delete_error: Option<&'static str>,
) -> Response {
    let page = SettingsPage {
        chrome: Chrome::new(),
        nav: Nav::of(user),
        slug,
        form,
        error,
        delete_error,
    };
    if error.is_some() || delete_error.is_some() {
        refused(&page)
    } else {
        html(&page)
    }
}

/// `GET /admin/p/{slug}/settings`.
pub async fn settings(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
    Path(slug): Path<String>,
) -> Response {
    match find(&app, &slug).await {
        Ok(Some(project)) => settings_page(
            &user,
            project.slug,
            SettingsForm::of(&project.settings),
            None,
            None,
        ),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(failure) => server_error(&failure),
    }
}

/// `POST /admin/p/{slug}/settings`: replaces every setting; an unchanged form records
/// nothing.
pub async fn save(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
    Path(slug): Path<String>,
    Form(form): Form<SettingsForm>,
) -> Response {
    let project = match find(&app, &slug).await {
        Ok(Some(project)) => project,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(failure) => return server_error(&failure),
    };
    let settings = match form.parse(&app) {
        Ok(settings) => settings,
        Err(problem) => {
            return settings_page(&user, project.slug, form, Some(problem.message), None);
        }
    };
    let (actor, now, id) = (actor(&user), app.clock.unix(), project.id);
    let updated = app
        .db
        .write(move |tx| projects::update(tx, id, &settings, actor, now))
        .await;
    match updated {
        Ok(changed) => {
            if changed {
                publish(&app).await;
            }
            to_settings(&project.slug)
        }
        Err(ProjectError::Invalid(problem)) => {
            settings_page(&user, project.slug, form, Some(problem.message), None)
        }
        Err(ProjectError::Database(failure)) => server_error(&failure),
    }
}

/// `POST /admin/p/{slug}/delete`: only with the slug typed exactly as confirmation.
pub async fn delete(
    State(app): State<AppState>,
    Extension(user): Extension<SessionUser>,
    Path(slug): Path<String>,
    Form(form): Form<DeleteForm>,
) -> Response {
    let project = match find(&app, &slug).await {
        Ok(Some(project)) => project,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(failure) => return server_error(&failure),
    };
    if form.confirm != project.slug {
        let current = SettingsForm::of(&project.settings);
        return settings_page(&user, project.slug, current, None, Some(CONFIRM_FAILED));
    }
    let (actor, now, id) = (actor(&user), app.clock.unix(), project.id);
    let deleted = app
        .db
        .write(move |tx| projects::delete(tx, id, actor, now).map_err(DbFailure::from))
        .await;
    match deleted {
        Ok(true) => {
            publish(&app).await;
            see_other(LIST_PATH)
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(failure) => server_error(&failure),
    }
}
