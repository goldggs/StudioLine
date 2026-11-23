pub mod agent;
mod db;
pub mod studio;
pub mod tunnel;
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Path, Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use db::Store;
use include_dir::{include_dir, Dir};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

static UI: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/../ui/dist");
pub type AppState = Arc<Store>;

/// Confirm the embedded interface has its entry point and built assets.
pub fn interface_is_embedded() -> bool {
    UI.get_file("index.html").is_some()
        && UI.get_dir("assets").is_some_and(|assets| {
            assets
                .files()
                .any(|file| file.path().extension().is_some_and(|ext| ext == "js"))
                && assets
                    .files()
                    .any(|file| file.path().extension().is_some_and(|ext| ext == "css"))
        })
}

pub fn app(
    store: AppState,
    port: u16,
    shutdown: Arc<tokio::sync::Notify>,
    bridge: Arc<studio::Bridge>,
) -> Router {
    app_with_agents(store, port, shutdown, bridge, None)
}

pub fn app_with_agents(
    store: AppState,
    port: u16,
    shutdown: Arc<tokio::sync::Notify>,
    bridge: Arc<studio::Bridge>,
    agents: Option<Arc<agent::Agents>>,
) -> Router {
    let router = Router::new()
        .route("/api/agents/start", axum::routing::post(agent::start))
        .route("/api/agents/status", get(agent::status))
        .route("/api/agents/stop", axum::routing::post(agent::stop))
        .route("/api/agents/issue", axum::routing::post(agent::issue))
        .route("/api/agents", get(agent::list))
        .route("/api/agents/{id}", axum::routing::delete(agent::revoke))
        .route("/api/health", get(health))
        .route("/api/studio/status", get(studio::status))
        .route(
            "/api/studio/activate",
            axum::routing::post(studio::activate),
        )
        .route(
            "/api/studio/install",
            get(studio::install_status).post(studio::install),
        )
        .route("/api/studio/inspect", axum::routing::post(studio::inspect))
        .route("/api/studio/socket", get(studio::plugin_socket))
        .route("/api/studio/events", get(studio::events_socket))
        .route(
            "/api/shutdown",
            axum::routing::post(move || async move {
                shutdown.notify_one();
                Json(serde_json::json!({"status": "shutting_down"}))
            }),
        )
        .route("/api/projects", get(list_projects).post(create_project))
        .route(
            "/api/projects/{id}",
            get(get_project).delete(delete_project),
        )
        .route("/api/export", axum::routing::post(export_data))
        .fallback(get(static_file))
        .layer(axum::Extension(bridge))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware::from_fn(move |req, next| {
            guard_local_request(req, next, port)
        }))
        .with_state(store);
    if let Some(agents) = agents {
        router.layer(axum::Extension(agents))
    } else {
        router
    }
}

pub fn open_store(path: &std::path::Path) -> Result<AppState, rusqlite::Error> {
    Ok(Arc::new(Store::open(path)?))
}

async fn guard_local_request(req: Request, next: Next, port: u16) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let valid_host = host.eq_ignore_ascii_case(&format!("localhost:{port}"))
        || host == format!("127.0.0.1:{port}");
    if !valid_host {
        return error(
            StatusCode::FORBIDDEN,
            "This address cannot access the local app.",
        );
    }
    if req.method() != axum::http::Method::GET && req.method() != axum::http::Method::HEAD {
        let marked = req
            .headers()
            .get("x-studioline-request")
            .is_some_and(|v| v == "1");
        let valid_origin = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .is_none_or(|origin| origin == format!("http://{host}"));
        if !marked || !valid_origin {
            return error(StatusCode::FORBIDDEN, "This request is not allowed.");
        }
    }
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        format!("default-src 'self'; connect-src 'self' ws://127.0.0.1:{port} ws://localhost:{port}; object-src 'none'; base-uri 'none'; frame-ancestors 'none'")
            .parse()
            .unwrap(),
    );
    response
}
#[derive(Serialize)]
struct ApiError {
    error: &'static str,
}
fn error(status: StatusCode, message: &'static str) -> Response {
    (status, Json(ApiError { error: message })).into_response()
}
fn internal_error() -> Response {
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "StudioLine could not access project storage. Check the local log and try again.",
    )
}
async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"app": "StudioLine", "status": "ready"}))
}
async fn list_projects(State(store): State<AppState>) -> Response {
    match tokio::task::spawn_blocking(move || store.list()).await {
        Ok(Ok(projects)) => Json(projects).into_response(),
        _ => {
            tracing::error!("Failed to list projects");
            internal_error()
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProject {
    name: String,
}
async fn create_project(
    State(store): State<AppState>,
    Json(input): Json<CreateProject>,
) -> Response {
    let name = match db::validate_name(&input.name) {
        Ok(name) => name.to_owned(),
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    match tokio::task::spawn_blocking(move || store.create(&name)).await {
        Ok(Ok(project)) => (StatusCode::CREATED, Json(project)).into_response(),
        Ok(Err(db::CreateError::Duplicate)) => error(
            StatusCode::CONFLICT,
            "A project with this name already exists.",
        ),
        Ok(Err(db::CreateError::Invalid(message))) => error(StatusCode::BAD_REQUEST, message),
        Ok(Err(db::CreateError::Database(e))) => {
            tracing::error!("Failed to create a project: {e}");
            internal_error()
        }
        _ => {
            tracing::error!("Failed to create a project");
            internal_error()
        }
    }
}
async fn get_project(State(store): State<AppState>, Path(id): Path<String>) -> Response {
    if uuid::Uuid::parse_str(&id).is_err() {
        return error(StatusCode::NOT_FOUND, "Project not found.");
    }
    match tokio::task::spawn_blocking(move || store.get(&id)).await {
        Ok(Ok(Some(project))) => Json(project).into_response(),
        Ok(Ok(None)) => error(StatusCode::NOT_FOUND, "Project not found."),
        _ => {
            tracing::error!("Failed to read a project");
            internal_error()
        }
    }
}
async fn delete_project(
    State(store): State<AppState>,
    axum::extract::Extension(bridge): axum::extract::Extension<Arc<studio::Bridge>>,
    agents: Option<axum::extract::Extension<Arc<agent::Agents>>>,
    Path(id): Path<String>,
) -> Response {
    if uuid::Uuid::parse_str(&id).is_err() {
        return error(StatusCode::NOT_FOUND, "Project not found.");
    }
    let project_id = id.clone();
    match tokio::task::spawn_blocking(move || bridge.delete_project(&store, &id)).await {
        Ok(Ok(true)) => {
            if let Some(axum::extract::Extension(agents)) = agents {
                agents.revoke_project(&project_id);
            }
            Json(serde_json::json!({"deleted": true})).into_response()
        }
        Ok(Ok(false)) => error(StatusCode::NOT_FOUND, "Project not found."),
        Ok(Err(message)) => error(StatusCode::CONFLICT, message),
        Err(_) => internal_error(),
    }
}
async fn export_data(State(store): State<AppState>) -> Response {
    match tokio::task::spawn_blocking(move || store.export()).await {
        Ok(Ok(bytes)) => (
            [
                (header::CONTENT_TYPE, "application/vnd.sqlite3"),
                (
                    header::CONTENT_DISPOSITION,
                    "attachment; filename=\"StudioLine-export.db\"",
                ),
            ],
            bytes,
        )
            .into_response(),
        Ok(Err(db::ExportError::Busy)) => error(
            StatusCode::CONFLICT,
            "StudioLine is working on data. Try exporting again when it finishes.",
        ),
        _ => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not export StudioLine data. Check disk space and try again.",
        ),
    }
}
async fn static_file(headers: HeaderMap, req: Request<Body>) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if path.starts_with("api/") {
        return error(StatusCode::NOT_FOUND, "Not found.");
    }
    match UI.get_file(path) {
        Some(file) => (
            [(
                header::CONTENT_TYPE,
                mime_guess::from_path(path).first_or_octet_stream().as_ref(),
            )],
            file.contents(),
        )
            .into_response(),
        None if headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("text/html"))
            && !path.contains('.') =>
        {
            match UI.get_file("index.html") {
                Some(file) => (
                    [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    file.contents(),
                )
                    .into_response(),
                None => error(StatusCode::NOT_FOUND, "Interface not found."),
            }
        }
        None => error(StatusCode::NOT_FOUND, "Not found."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    #[test]
    fn built_interface_is_embedded() {
        assert!(interface_is_embedded());
    }
    fn test_bridge() -> Arc<studio::Bridge> {
        let dir = tempfile::tempdir().unwrap();
        Arc::new(studio::Bridge::new(dir.path(), None).unwrap())
    }
    fn test_app() -> Router {
        app(
            Arc::new(Store::memory().unwrap()),
            38741,
            Arc::new(tokio::sync::Notify::new()),
            test_bridge(),
        )
    }
    async fn send(
        method: &str,
        uri: &str,
        host: &str,
        origin: Option<&str>,
        marker: bool,
        body: &str,
    ) -> Response {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("host", host)
            .header("content-type", "application/json");
        if let Some(origin) = origin {
            builder = builder.header("origin", origin);
        }
        if marker {
            builder = builder.header("x-studioline-request", "1");
        }
        test_app()
            .oneshot(builder.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap()
    }
    #[tokio::test]
    async fn blocks_host_rebinding_and_unmarked_or_cross_origin_writes() {
        assert_eq!(
            send("GET", "/api/projects", "evil.test:38741", None, false, "")
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(
                "POST",
                "/api/projects",
                "127.0.0.1:38741",
                None,
                false,
                "{\"name\":\"x\"}"
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(
                "POST",
                "/api/projects",
                "127.0.0.1:38741",
                Some("https://evil.test"),
                true,
                "{\"name\":\"x\"}"
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            send(
                "POST",
                "/api/projects",
                "127.0.0.1:38741",
                Some("http://127.0.0.1:38741"),
                true,
                "{\"name\":\"x\"}"
            )
            .await
            .status(),
            StatusCode::CREATED
        );
    }
    #[tokio::test]
    async fn rejects_bad_names_and_unknown_payload_fields() {
        assert_eq!(
            send(
                "POST",
                "/api/projects",
                "localhost:38741",
                None,
                true,
                "{\"name\":\"  \"}"
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            send(
                "POST",
                "/api/projects",
                "localhost:38741",
                None,
                true,
                "{\"name\":\"a\",\"admin\":true}"
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            send(
                "GET",
                "/api/projects/not-a-uuid",
                "localhost:38741",
                None,
                false,
                ""
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
    }
    #[tokio::test]
    async fn rejects_oversized_requests_and_unsafe_hosts() {
        let huge = format!("{{\"name\":\"{}\"}}", "x".repeat(17000));
        assert_eq!(
            send(
                "POST",
                "/api/projects",
                "localhost:38741",
                None,
                true,
                &huge
            )
            .await
            .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        for host in [
            "localhost.evil.test:38741",
            "127.0.0.1.evil.test:38741",
            "127.0.0.1:80",
            "",
        ] {
            assert_eq!(
                send("GET", "/api/projects", host, None, false, "")
                    .await
                    .status(),
                StatusCode::FORBIDDEN
            );
        }
    }
    #[tokio::test]
    async fn concurrent_creates_are_all_stored() {
        let service = test_app();
        let mut tasks = Vec::new();
        for i in 0..20 {
            let svc = service.clone();
            tasks.push(tokio::spawn(async move {
                let request = Request::builder()
                    .method("POST")
                    .uri("/api/projects")
                    .header("host", "127.0.0.1:38741")
                    .header("content-type", "application/json")
                    .header("x-studioline-request", "1")
                    .body(Body::from(format!("{{\"name\":\"Project {i}\"}}")))
                    .unwrap();
                svc.oneshot(request).await.unwrap().status()
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap(), StatusCode::CREATED);
        }
        let response = service
            .oneshot(
                Request::builder()
                    .uri("/api/projects")
                    .header("host", "127.0.0.1:38741")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let projects: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(projects.len(), 20);
    }
    #[tokio::test]
    async fn delete_and_export_require_local_authorization() {
        let service = test_app();
        for (method, path) in [
            (
                "DELETE",
                "/api/projects/00000000-0000-0000-0000-000000000000",
            ),
            ("POST", "/api/export"),
        ] {
            for (marked, origin) in [
                (false, "http://127.0.0.1:38741"),
                (true, "http://evil.test"),
            ] {
                let mut request = Request::builder()
                    .method(method)
                    .uri(path)
                    .header("host", "127.0.0.1:38741")
                    .header("origin", origin);
                if marked {
                    request = request.header("x-studioline-request", "1");
                }
                let response = service
                    .clone()
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::FORBIDDEN);
            }
        }
    }
    #[tokio::test]
    async fn export_response_is_sqlite_attachment() {
        let service = test_app();
        let request = Request::builder()
            .method("POST")
            .uri("/api/export")
            .header("host", "127.0.0.1:38741")
            .header("x-studioline-request", "1")
            .body(Body::empty())
            .unwrap();
        let response = service.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains("StudioLine-export.db"));
        let body = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert!(body.starts_with(b"SQLite format 3\0"));
    }
    #[tokio::test]
    async fn duplicate_project_request_returns_conflict() {
        let service = test_app();
        let make_request = |name: &str| {
            Request::builder()
                .method("POST")
                .uri("/api/projects")
                .header("host", "127.0.0.1:38741")
                .header("content-type", "application/json")
                .header("x-studioline-request", "1")
                .body(Body::from(serde_json::json!({"name": name}).to_string()))
                .unwrap()
        };
        let first = service
            .clone()
            .oneshot(make_request("Project"))
            .await
            .unwrap();
        let second = service.oneshot(make_request(" project ")).await.unwrap();
        assert_eq!(first.status(), StatusCode::CREATED);
        assert_eq!(second.status(), StatusCode::CONFLICT);
    }
    #[tokio::test]
    async fn static_assets_and_api_routes_are_distinct() {
        assert_eq!(
            send("GET", "/", "localhost:38741", None, false, "")
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            send("GET", "/api/unknown", "localhost:38741", None, false, "")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send("GET", "/missing.js", "localhost:38741", None, false, "")
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    #[tokio::test]
    async fn shutdown_requires_a_same_origin_marked_post() {
        let shutdown = Arc::new(tokio::sync::Notify::new());
        let service = app(
            Arc::new(Store::memory().unwrap()),
            38741,
            shutdown.clone(),
            test_bridge(),
        );
        let request = |origin: &str, marked: bool| {
            let mut req = Request::builder()
                .method("POST")
                .uri("/api/shutdown")
                .header("host", "127.0.0.1:38741")
                .header("origin", origin);
            if marked {
                req = req.header("x-studioline-request", "1");
            }
            req.body(Body::empty()).unwrap()
        };
        assert_eq!(
            service
                .clone()
                .oneshot(request("https://evil.example", true))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), shutdown.notified())
                .await
                .is_err()
        );
        assert_eq!(
            service
                .clone()
                .oneshot(request("http://127.0.0.1:38741", false))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            service
                .oneshot(request("http://127.0.0.1:38741", true))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), shutdown.notified())
                .await
                .is_ok()
        );
    }
}
