//! Narrow agent-only API, served on a separate loopback port from local administration.
use crate::{studio::Bridge, tunnel::Tunnel};
use axum::{
    extract::{DefaultBodyLimit, Extension, Json, Path},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use uuid::Uuid;

struct Record {
    hash: [u8; 32],
    project_id: String,
    name: Option<String>,
    role: Option<String>,
    window: Instant,
    requests: u32,
}
pub struct Agents {
    bridge: Arc<Bridge>,
    tunnel: Arc<Tunnel>,
    records: Mutex<HashMap<String, Record>>,
}
#[derive(Serialize)]
struct AgentSummary {
    id: String,
    name: Option<String>,
    role: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRequest {
    project_id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    name: String,
    role: String,
}
impl Agents {
    pub fn new(bridge: Arc<Bridge>, tunnel: Arc<Tunnel>) -> Arc<Self> {
        Arc::new(Self {
            bridge,
            tunnel,
            records: Mutex::new(HashMap::new()),
        })
    }
    fn issue(&self, project: &str) -> Result<(String, String), &'static str> {
        if self.tunnel.status().state != "ready" {
            return Err("Start the agent connection first.");
        }
        if self.bridge.status().project_id.as_deref() != Some(project) {
            return Err("Open this project first.");
        }
        let mut records = self
            .records
            .lock()
            .map_err(|_| "Agent connections are unavailable.")?;
        if records.len() >= 32 {
            return Err("Too many agent credentials. Revoke one first.");
        }
        let id = Uuid::new_v4().to_string();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let hash: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        records.insert(
            id.clone(),
            Record {
                hash,
                project_id: project.into(),
                name: None,
                role: None,
                window: Instant::now(),
                requests: 0,
            },
        );
        Ok((id, token))
    }
    fn authenticate(&self, headers: &HeaderMap) -> Result<String, StatusCode> {
        if headers.contains_key(header::ORIGIN) {
            return Err(StatusCode::FORBIDDEN);
        }
        let token = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(StatusCode::UNAUTHORIZED)?;
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        if self.tunnel.status().state != "ready" {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let digest = Sha256::digest(token.as_bytes());
        let mut records = self
            .records
            .lock()
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        // Iterate over all records and compare digests in constant time; never log tokens.
        let mut matched = None;
        for (id, record) in records.iter() {
            if bool::from(record.hash.as_slice().ct_eq(digest.as_slice())) {
                matched = Some(id.clone());
            }
        }
        let id = matched.ok_or(StatusCode::UNAUTHORIZED)?;
        let record = records.get_mut(&id).ok_or(StatusCode::UNAUTHORIZED)?;
        if record.window.elapsed() >= Duration::from_secs(60) {
            record.window = Instant::now();
            record.requests = 0;
        }
        if record.requests >= 120 {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        record.requests += 1;
        Ok(id)
    }
    pub fn shutdown(&self) {
        self.clear();
        self.tunnel.stop();
    }
    pub(crate) fn revoke_project(&self, project: &str) {
        if let Ok(mut records) = self.records.lock() {
            records.retain(|_, record| record.project_id != project);
        }
    }
    fn clear(&self) {
        if let Ok(mut records) = self.records.lock() {
            records.clear();
        }
    }
}
fn fail(status: StatusCode, message: &'static str) -> Response {
    (status, Json(serde_json::json!({"error": message}))).into_response()
}
fn local(agents: Option<Extension<Arc<Agents>>>) -> Option<Arc<Agents>> {
    agents.map(|Extension(a)| a)
}

pub async fn start(
    agents: Option<Extension<Arc<Agents>>>,
    Json(input): Json<ProjectRequest>,
) -> Response {
    let agents = match local(agents) {
        Some(a) => a,
        None => {
            return fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent connections are unavailable.",
            )
        }
    };
    if agents.bridge.status().project_id.as_deref() != Some(&input.project_id) {
        return fail(StatusCode::CONFLICT, "Open this project first.");
    }
    if matches!(agents.tunnel.status().state, "stopped" | "error") {
        agents.clear();
    }
    match agents.tunnel.start() {
        Ok(()) => Json(agents.tunnel.status()).into_response(),
        Err(message) => fail(StatusCode::CONFLICT, message),
    }
}
pub async fn status(agents: Option<Extension<Arc<Agents>>>) -> Response {
    match local(agents) {
        Some(a) => Json(a.tunnel.status()).into_response(),
        None => fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent connections are unavailable.",
        ),
    }
}
pub async fn stop(agents: Option<Extension<Arc<Agents>>>) -> Response {
    match local(agents) {
        Some(a) => {
            a.clear();
            a.tunnel.stop();
            Json(serde_json::json!({"stopped":true})).into_response()
        }
        None => fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent connections are unavailable.",
        ),
    }
}
pub async fn issue(
    agents: Option<Extension<Arc<Agents>>>,
    Json(input): Json<ProjectRequest>,
) -> Response {
    let agents = match local(agents) {
        Some(a) => a,
        None => {
            return fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent connections are unavailable.",
            )
        }
    };
    match agents.issue(&input.project_id) {
        Ok((id, token)) => Json(serde_json::json!({"id":id,"token":token})).into_response(),
        Err(message) => fail(StatusCode::CONFLICT, message),
    }
}
pub async fn list(
    agents: Option<Extension<Arc<Agents>>>,
    axum::extract::Query(input): axum::extract::Query<ProjectRequest>,
) -> Response {
    let agents = match local(agents) {
        Some(a) => a,
        None => {
            return fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent connections are unavailable.",
            )
        }
    };
    let Ok(records) = agents.records.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent connections are unavailable.",
        );
    };
    let items: Vec<_> = records
        .iter()
        .filter(|(_, r)| r.project_id == input.project_id)
        .map(|(id, r)| AgentSummary {
            id: id.clone(),
            name: r.name.clone(),
            role: r.role.clone(),
        })
        .collect();
    Json(items).into_response()
}
pub async fn revoke(agents: Option<Extension<Arc<Agents>>>, Path(id): Path<String>) -> Response {
    let agents = match local(agents) {
        Some(a) => a,
        None => {
            return fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent connections are unavailable.",
            )
        }
    };
    let Ok(mut records) = agents.records.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent connections are unavailable.",
        );
    };
    if records.remove(&id).is_some() {
        Json(serde_json::json!({"revoked":true})).into_response()
    } else {
        fail(StatusCode::NOT_FOUND, "Agent credential not found.")
    }
}
fn valid_text(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max && !s.chars().any(char::is_control)
}
async fn register(
    Extension(agents): Extension<Arc<Agents>>,
    headers: HeaderMap,
    Json(input): Json<Registration>,
) -> Response {
    let id = match agents.authenticate(&headers) {
        Ok(id) => id,
        Err(code) => return fail(code, "Agent authentication failed or unavailable."),
    };
    if !valid_text(&input.name, 80) || !valid_text(&input.role, 120) {
        return fail(
            StatusCode::BAD_REQUEST,
            "Provide a name and role without control characters.",
        );
    }
    let Ok(mut records) = agents.records.lock() else {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "Agent connections are unavailable.",
        );
    };
    let Some(record) = records.get_mut(&id) else {
        return fail(StatusCode::UNAUTHORIZED, "Agent credential was revoked.");
    };
    if record.name.is_some() {
        if record.name.as_deref() == Some(input.name.trim())
            && record.role.as_deref() == Some(input.role.trim())
        {
            return Json(serde_json::json!({"registered": true, "agent_id": id})).into_response();
        }
        return fail(
            StatusCode::CONFLICT,
            "This credential is already registered.",
        );
    }
    record.name = Some(input.name.trim().into());
    record.role = Some(input.role.trim().into());
    Json(serde_json::json!({"registered":true,"agent_id":id})).into_response()
}
async fn inspect(Extension(agents): Extension<Arc<Agents>>, headers: HeaderMap) -> Response {
    let id = match agents.authenticate(&headers) {
        Ok(id) => id,
        Err(code) => return fail(code, "Agent authentication failed or unavailable."),
    };
    let project = {
        let Ok(records) = agents.records.lock() else {
            return fail(
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent connections are unavailable.",
            );
        };
        let Some(record) = records.get(&id) else {
            return fail(StatusCode::UNAUTHORIZED, "Agent credential was revoked.");
        };
        if record.name.is_none() {
            return fail(
                StatusCode::FORBIDDEN,
                "Register this agent before using commands.",
            );
        }
        record.project_id.clone()
    };
    match agents.bridge.inspect(&project).await {
        Ok(info) => Json(info).into_response(),
        Err(message) => fail(StatusCode::CONFLICT, message),
    }
}
/// This router is the ONLY service forwarded by the tunnel. It contains no local admin or UI routes.
pub fn public_router(agents: Arc<Agents>) -> Router {
    let slots = Arc::new(tokio::sync::Semaphore::new(32));
    Router::new()
        .route("/v1/register", post(register))
        .route("/v1/inspect", post(inspect))
        .fallback(get(|| async { StatusCode::NOT_FOUND }))
        .layer(DefaultBodyLimit::max(4096))
        .layer(Extension(agents))
        .layer(axum::middleware::from_fn(
            move |request: axum::extract::Request, next: axum::middleware::Next| {
                let slots = slots.clone();
                async move {
                    let Ok(_permit) = slots.try_acquire_owned() else {
                        return fail(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "Agent service is busy. Try again.",
                        );
                    };
                    let mut response = match tokio::time::timeout(
                        Duration::from_secs(15),
                        next.run(request),
                    )
                    .await
                    {
                        Ok(response) => response,
                        Err(_) => {
                            fail(StatusCode::GATEWAY_TIMEOUT, "Command timed out. Try again.")
                        }
                    };
                    response
                        .headers_mut()
                        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
                    response
                        .headers_mut()
                        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
                    response
                }
            },
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    #[tokio::test]
    async fn isolated_public_api_rejects_invalid_credentials_and_revocation_is_effective() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = Arc::new(Bridge::new(dir.path(), None).unwrap());
        let store = crate::db::Store::memory().unwrap();
        let project = store.create("Test").unwrap();
        bridge
            .activate_existing(&store, project.id.clone())
            .unwrap();
        let tunnel = Tunnel::new(dir.path().to_path_buf(), 12345);
        tunnel.test_ready();
        let agents = Agents::new(bridge, tunnel);
        let (id, token) = agents.issue(&project.id).unwrap();
        assert_ne!(token, id);
        assert_eq!(token.len(), 64);
        let service = public_router(agents.clone());
        let req = |method: &str, path: &str, token: &str, origin: bool, body: &str| {
            let mut builder = Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json");
            if !token.is_empty() {
                builder = builder.header("authorization", format!("Bearer {token}"));
            }
            if origin {
                builder = builder.header("origin", "https://evil.example");
            }
            builder.body(Body::from(body.to_owned())).unwrap()
        };
        assert_eq!(
            service
                .clone()
                .oneshot(req("POST", "/v1/inspect", "", false, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            service
                .clone()
                .oneshot(req("POST", "/v1/inspect", &"a".repeat(64), false, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            service
                .clone()
                .oneshot(req("POST", "/v1/inspect", &token, true, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            service
                .clone()
                .oneshot(req("POST", "/v1/inspect", &token, false, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            service
                .clone()
                .oneshot(req(
                    "POST",
                    "/v1/register",
                    &token,
                    false,
                    r#"{"name":"Agent","role":"Builder"}"#
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            service
                .clone()
                .oneshot(req(
                    "POST",
                    "/v1/register",
                    &token,
                    false,
                    r#"{"name":"Other","role":"Builder"}"#
                ))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            service
                .clone()
                .oneshot(req("POST", "/v1/inspect", &token, false, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::CONFLICT
        );
        for path in [
            "/",
            "/api/projects",
            "/api/shutdown",
            "/api/agents/issue",
            "/api/studio/socket",
        ] {
            assert_ne!(
                service
                    .clone()
                    .oneshot(req("POST", path, &token, false, ""))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
        }
        assert_eq!(
            agents.records.lock().unwrap()[&id].name.as_deref(),
            Some("Agent")
        );
        agents.revoke_project(&project.id);
        assert_eq!(
            service
                .oneshot(req("POST", "/v1/inspect", &token, false, ""))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
