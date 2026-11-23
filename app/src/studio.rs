//! Local, authenticated Studio plugin connection. This module never exposes its key to the browser.
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Json,
    },
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, oneshot, watch};
use uuid::Uuid;

const PLUGIN_SOURCE: &str = include_str!("../../plugin/src/StudioLine.server.luau");
const PLUGIN_ICON: &[u8] = include_bytes!("../../design/StudioLine-logo.png");
const KEY_MARKER: &str = "__STUDIOLINE_PLUGIN_KEY__";

#[derive(Clone, Debug, Default, Serialize)]
pub struct StudioStatus {
    pub project_id: Option<String>,
    pub connected: bool,
    pub place_name: Option<String>,
    pub experience_name: Option<String>,
    pub place_id: Option<u64>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Child {
    name: String,
    class_name: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceInfo {
    name: String,
    #[serde(default)]
    experience_name: Option<String>,
    place_id: u64,
    workspace_children: Vec<Child>,
    total_children: u64,
}
struct Session {
    id: String,
    sender: mpsc::Sender<String>,
    pending: HashMap<String, oneshot::Sender<Result<PlaceInfo, &'static str>>>,
}
#[derive(Default)]
struct Inner {
    shutting_down: bool,
    status: StudioStatus,
    session: Option<Session>,
}
pub struct Bridge {
    key: String,
    plugin_dir: Option<PathBuf>,
    inner: Mutex<Inner>,
    updates: watch::Sender<StudioStatus>,
    shutdown_signal: watch::Sender<bool>,
}

impl Bridge {
    pub fn new(data_dir: &Path, plugin_dir: Option<PathBuf>) -> std::io::Result<Self> {
        let key = load_or_create_key(data_dir)?;
        let (updates, _) = watch::channel(StudioStatus::default());
        let (shutdown_signal, _) = watch::channel(false);
        Ok(Self {
            key,
            plugin_dir,
            inner: Mutex::new(Inner::default()),
            updates,
            shutdown_signal,
        })
    }
    pub(crate) fn status(&self) -> StudioStatus {
        self.updates.borrow().clone()
    }
    fn publish(&self, inner: &Inner) {
        self.updates.send_replace(inner.status.clone());
    }
    #[cfg(test)]
    fn activate(&self, project_id: String) -> Result<(), &'static str> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Studio connection is unavailable.")?;
        if inner.session.is_some() && inner.status.project_id.as_deref() != Some(&project_id) {
            return Err("Disconnect Studio before opening another project.");
        }
        inner.status.project_id = Some(project_id);
        self.publish(&inner);
        Ok(())
    }
    pub(crate) fn activate_existing(
        &self,
        store: &super::Store,
        project_id: String,
    ) -> Result<bool, &'static str> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Studio connection is unavailable.")?;
        if inner.shutting_down {
            return Err("StudioLine is shutting down.");
        }
        if inner.session.is_some() && inner.status.project_id.as_deref() != Some(&project_id) {
            return Err("Disconnect Studio before opening another project.");
        }
        if store
            .get(&project_id)
            .map_err(|_| "Could not access project storage.")?
            .is_none()
        {
            return Ok(false);
        }
        inner.status.project_id = Some(project_id);
        self.publish(&inner);
        Ok(true)
    }
    pub(crate) fn delete_project(
        &self,
        store: &super::Store,
        project_id: &str,
    ) -> Result<bool, &'static str> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Studio connection is unavailable.")?;
        if inner.shutting_down {
            return Err("StudioLine is shutting down.");
        }
        if inner.status.project_id.as_deref() == Some(project_id) && inner.session.is_some() {
            return Err("Disconnect Studio before deleting this project.");
        }
        let deleted = store
            .delete(project_id)
            .map_err(|_| "Could not delete the project. Check storage and try again.")?;
        if deleted && inner.status.project_id.as_deref() == Some(project_id) {
            inner.status.project_id = None;
            self.publish(&inner);
        }
        Ok(deleted)
    }
    fn start_session(
        &self,
        place_name: String,
        place_id: u64,
        sender: mpsc::Sender<String>,
    ) -> Result<String, &'static str> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| "Studio connection is unavailable.")?;
        if inner.shutting_down {
            return Err("StudioLine is shutting down. Open the app and try again.");
        }
        if inner.status.project_id.is_none() {
            return Err("Open a project in StudioLine first, then connect again.");
        }
        if inner.session.is_some() {
            return Err("Another Studio session is already connected.");
        }
        let id = Uuid::new_v4().to_string();
        inner.session = Some(Session {
            id: id.clone(),
            sender,
            pending: HashMap::new(),
        });
        inner.status.connected = true;
        inner.status.place_name = Some(place_name);
        inner.status.place_id = Some(place_id);
        self.publish(&inner);
        Ok(id)
    }
    /// Ask an authenticated Studio session to stop reconnecting before the app exits.
    /// A broken connection cannot acknowledge, so shutdown remains bounded.
    pub async fn prepare_shutdown(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.shutting_down = true;
            self.shutdown_signal.send_replace(true);
        }
        let mut status = self.updates.subscribe();
        let _ = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if !status.borrow_and_update().connected {
                    break;
                }
                if status.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
    }
    fn end_session(&self, id: &str) {
        if let Ok(mut inner) = self.inner.lock() {
            if inner.session.as_ref().is_some_and(|s| s.id == id) {
                if let Some(session) = inner.session.take() {
                    for (_, pending) in session.pending {
                        let _ =
                            pending.send(Err("Studio disconnected before the command finished."));
                    }
                }
                inner.status.connected = false;
                inner.status.place_name = None;
                inner.status.experience_name = None;
                inner.status.place_id = None;
                self.publish(&inner);
            }
        }
    }
    fn update_experience_name(&self, session_id: &str, name: String) {
        if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
            return;
        }
        if let Ok(mut inner) = self.inner.lock() {
            if inner
                .session
                .as_ref()
                .is_some_and(|session| session.id == session_id)
            {
                inner.status.experience_name = Some(name);
                self.publish(&inner);
            }
        }
    }
    fn update_place_name(&self, session_id: &str, name: String) {
        if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
            return;
        }
        if let Ok(mut inner) = self.inner.lock() {
            if inner
                .session
                .as_ref()
                .is_some_and(|session| session.id == session_id)
            {
                inner.status.place_name = Some(name);
                self.publish(&inner);
            }
        }
    }
    fn deliver(&self, session_id: &str, result: PluginResult) {
        if let Ok(mut inner) = self.inner.lock() {
            let experience_name = inner.status.experience_name.clone();
            if let Some(session) = inner.session.as_mut().filter(|s| s.id == session_id) {
                if let Some(waiting) = session.pending.remove(&result.id) {
                    let value = match (result.ok, result.data) {
                        (true, Some(mut data))
                            if data.workspace_children.len() <= 25
                                && data.name.chars().count() <= 120
                                && data.workspace_children.iter().all(|c| {
                                    c.name.chars().count() <= 120 && c.class_name.len() <= 100
                                }) =>
                        {
                            data.experience_name = experience_name;
                            Ok(data)
                        }
                        _ => Err("Studio could not inspect the game."),
                    };
                    let _ = waiting.send(value);
                }
            }
        }
    }
    pub(crate) async fn inspect(&self, project_id: &str) -> Result<PlaceInfo, &'static str> {
        let id = Uuid::new_v4().to_string();
        let (sender, receiver) = oneshot::channel();
        {
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| "Studio connection is unavailable.")?;
            if inner.status.project_id.as_deref() != Some(project_id) {
                return Err("Open this project before inspecting Studio.");
            }
            let session = inner
                .session
                .as_mut()
                .ok_or("Studio is not connected. Open Studio and click Connect in the plugin.")?;
            if session.pending.len() >= 16 {
                return Err("Too many commands are already waiting. Try again in a moment.");
            }
            let command = serde_json::json!({"type":"command", "command":"inspect_place", "id":id})
                .to_string();
            session.pending.insert(id.clone(), sender);
            if session.sender.try_send(command).is_err() {
                session.pending.remove(&id);
                return Err("Studio is busy or disconnected. Try again.");
            }
        }
        let result = match tokio::time::timeout(Duration::from_secs(10), receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Studio disconnected before the command finished."),
            Err(_) => {
                Err("Studio did not respond in time. Check the plugin connection and try again.")
            }
        };
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(session) = inner.session.as_mut() {
                session.pending.remove(&id);
            }
        }
        result
    }
    fn plugin_icon_path(directory: &Path) -> Result<PathBuf, &'static str> {
        let roblox = directory
            .parent()
            .ok_or("Could not find the Roblox local assets folder.")?;
        Ok(roblox.join("LocalAssets").join("StudioLine-logo.png"))
    }
    fn icon_is_current(directory: &Path) -> bool {
        let Ok(path) = Self::plugin_icon_path(directory) else {
            return false;
        };
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return false;
        };
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() == PLUGIN_ICON.len() as u64
            && fs::read(path).is_ok_and(|icon| icon == PLUGIN_ICON)
    }
    fn install_plugin_icon(directory: &Path) -> Result<(), &'static str> {
        let path = Self::plugin_icon_path(directory)?;
        let parent = path
            .parent()
            .ok_or("Could not find the Roblox local assets folder.")?;
        fs::create_dir_all(parent)
            .map_err(|_| "Could not create the Roblox local assets folder.")?;
        // Never replace a file we do not recognize, or follow a link supplied at this path.
        match fs::symlink_metadata(&path) {
            Ok(metadata)
                if !metadata.file_type().is_file() || metadata.file_type().is_symlink() =>
            {
                return Err(
                    "The StudioLine icon path is not a regular file. Inspect it before retrying.",
                )
            }
            Ok(_) => {
                return if Self::icon_is_current(directory) {
                    Ok(())
                } else {
                    Err("A different StudioLine-logo.png already exists in Roblox LocalAssets. It was not overwritten.")
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Could not check the StudioLine plugin icon."),
        }
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|_| "Could not write the StudioLine plugin icon.")?;
        temporary
            .write_all(PLUGIN_ICON)
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|_| "Could not write the StudioLine plugin icon.")?;
        temporary.persist_noclobber(&path).map_err(|_| "Could not install the StudioLine plugin icon. Check folder permissions or a conflicting file.")?;
        Ok(())
    }
    fn plugin_install_state(&self) -> Result<&'static str, &'static str> {
        let directory = self
            .plugin_dir
            .as_ref()
            .ok_or("Plugin installation is available on Windows only.")?;
        let path = directory.join("StudioLine.rbxmx");
        let expected =
            plugin_xml(&self.key).ok_or("The bundled plugin is invalid. Reinstall StudioLine.")?;
        match fs::read(&path) {
            Ok(actual) if actual == expected.as_bytes() && Self::icon_is_current(directory) => {
                Ok("installed")
            }
            Ok(_) => Ok("update_available"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok("missing"),
            Err(_) => {
                Err("Could not check the Studio plugin. Check folder permissions and try again.")
            }
        }
    }
    pub fn verify_plugin(&self) -> Result<(), &'static str> {
        let directory = self
            .plugin_dir
            .as_ref()
            .ok_or("Plugin installation is available on Windows only.")?;
        let expected =
            plugin_xml(&self.key).ok_or("The bundled plugin is invalid. Reinstall StudioLine.")?;
        let actual = fs::read(directory.join("StudioLine.rbxmx")).map_err(|_| {
            "The StudioLine plugin file is missing. Install the plugin and try again."
        })?;
        if actual != expected.as_bytes() || !Self::icon_is_current(directory) {
            return Err(
                "The StudioLine plugin does not match this installation. Install the plugin again.",
            );
        }
        Ok(())
    }
    pub fn install_plugin(&self) -> Result<(), &'static str> {
        if self.status().connected {
            return Err("Disconnect Studio before reinstalling the plugin.");
        }
        let directory = self
            .plugin_dir
            .as_ref()
            .ok_or("Plugin installation is available on Windows only.")?;
        fs::create_dir_all(directory)
            .map_err(|_| "Could not create the Studio plugin folder. Check folder permissions.")?;
        Self::install_plugin_icon(directory)?;
        let document =
            plugin_xml(&self.key).ok_or("The bundled plugin is invalid. Reinstall StudioLine.")?;
        let mut temporary = tempfile::NamedTempFile::new_in(directory).map_err(|_| {
            "Could not write to the Studio plugin folder. Check folder permissions."
        })?;
        temporary
            .write_all(document.as_bytes())
            .and_then(|_| temporary.as_file().sync_all())
            .map_err(|_| {
                "Could not write the plugin file. Check disk space and folder permissions."
            })?;
        temporary
            .persist(directory.join("StudioLine.rbxmx"))
            .map_err(|_| "Could not replace the StudioLine plugin. Close Studio and try again.")?;
        Ok(())
    }
}
fn load_or_create_key(data: &Path) -> std::io::Result<String> {
    let path = data.join("studio-plugin.key");
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            let key = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
            if let Err(error) = file.write_all(key.as_bytes()).and_then(|_| file.sync_all()) {
                let _ = fs::remove_file(&path);
                return Err(error);
            }
            Ok(key)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let mut key = String::new();
            fs::File::open(&path)?.take(65).read_to_string(&mut key)?;
            if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid local plugin key",
                ));
            }
            Ok(key)
        }
        Err(error) => Err(error),
    }
}
fn plugin_xml(key: &str) -> Option<String> {
    if PLUGIN_SOURCE.matches(KEY_MARKER).count() != 1 {
        return None;
    }
    let source = PLUGIN_SOURCE
        .replace(KEY_MARKER, key)
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    Some(format!("<roblox version=\"4\"><External>null</External><External>nil</External><Item class=\"Script\" referent=\"RBX0\"><Properties><string name=\"Name\">StudioLine</string><ProtectedString name=\"Source\">{source}</ProtectedString><bool name=\"Disabled\">false</bool></Properties></Item></roblox>"))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    #[serde(rename = "type")]
    kind: String,
    token: String,
    place_name: String,
    place_id: u64,
    #[serde(default)]
    game_id: Option<u64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginName {
    #[serde(rename = "type")]
    kind: String,
    name: String,
}
#[derive(Deserialize)]
struct PluginResult {
    #[serde(rename = "type")]
    kind: String,
    id: String,
    ok: bool,
    data: Option<PlaceInfo>,
}

pub async fn status(Extension(bridge): Extension<Arc<Bridge>>) -> Json<StudioStatus> {
    Json(bridge.status())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRequest {
    project_id: String,
}
pub async fn activate(
    Extension(bridge): Extension<Arc<Bridge>>,
    axum::extract::State(store): axum::extract::State<super::AppState>,
    Json(request): Json<ProjectRequest>,
) -> Response {
    let id = request.project_id;
    let worker_bridge = bridge.clone();
    let result =
        tokio::task::spawn_blocking(move || worker_bridge.activate_existing(&store, id)).await;
    match result {
        Ok(Ok(true)) => Json(bridge.status()).into_response(),
        Ok(Ok(false)) => super::error(StatusCode::NOT_FOUND, "Project not found."),
        Ok(Err(message)) => super::error(StatusCode::CONFLICT, message),
        Err(_) => super::internal_error(),
    }
}
pub async fn install_status(Extension(bridge): Extension<Arc<Bridge>>) -> Response {
    match tokio::task::spawn_blocking(move || bridge.plugin_install_state()).await {
        Ok(Ok(state)) => Json(serde_json::json!({"state": state})).into_response(),
        Ok(Err(message)) => super::error(StatusCode::INTERNAL_SERVER_ERROR, message),
        Err(_) => super::error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not check the Studio plugin. Try again.",
        ),
    }
}
pub async fn install(Extension(bridge): Extension<Arc<Bridge>>) -> Response {
    match tokio::task::spawn_blocking(move || bridge.install_plugin()).await {
        Ok(Ok(())) => Json(serde_json::json!({"installed": true})).into_response(),
        Ok(Err(message)) => super::error(StatusCode::INTERNAL_SERVER_ERROR, message),
        Err(_) => super::error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not install the Studio plugin. Try again.",
        ),
    }
}
pub async fn inspect(
    Extension(bridge): Extension<Arc<Bridge>>,
    Json(request): Json<ProjectRequest>,
) -> Response {
    match bridge.inspect(&request.project_id).await {
        Ok(info) => Json(info).into_response(),
        Err(message) => super::error(StatusCode::CONFLICT, message),
    }
}
pub async fn plugin_socket(
    headers: HeaderMap,
    Extension(bridge): Extension<Arc<Bridge>>,
    ws: WebSocketUpgrade,
) -> Response {
    // Browsers always include Origin on WebSocket requests. Only a non-browser client can use this route.
    if headers.contains_key(header::ORIGIN) {
        return super::error(StatusCode::FORBIDDEN, "This connection is not allowed.");
    }
    ws.max_message_size(16 * 1024)
        .max_frame_size(16 * 1024)
        .on_upgrade(move |socket| plugin_session(socket, bridge))
}
async fn plugin_session(mut socket: WebSocket, bridge: Arc<Bridge>) {
    let received = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await;
    let Some(Ok(Message::Text(text))) = received.ok().flatten() else {
        return;
    };
    let Ok(hello) = serde_json::from_str::<Hello>(&text) else {
        return;
    };
    if hello.kind != "hello"
        || hello.token.len() != bridge.key.len()
        || !bool::from(hello.token.as_bytes().ct_eq(bridge.key.as_bytes()))
        || hello.place_name.chars().count() > 120
    {
        let _ = socket.send(Message::Text(serde_json::json!({"type":"error", "message":"Plugin authentication failed. Reinstall the plugin from StudioLine."}).to_string().into())).await;
        return;
    }
    let (sender, mut receiver) = mpsc::channel::<String>(16);
    let mut shutdown = bridge.shutdown_signal.subscribe();
    let id = match bridge.start_session(hello.place_name, hello.place_id, sender) {
        Ok(id) => id,
        Err(message) => {
            let _ = socket
                .send(Message::Text(
                    serde_json::json!({"type":"error", "message":message})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };
    if let Some(game_id) = hello.game_id.filter(|id| *id > 0) {
        let bridge = bridge.clone();
        let session_id = id.clone();
        tokio::spawn(async move {
            if let Some(name) = lookup_experience_name(game_id).await {
                bridge.update_experience_name(&session_id, name);
            }
        });
    }
    if socket
        .send(Message::Text("{\"type\":\"connected\"}".into()))
        .await
        .is_ok()
    {
        loop {
            tokio::select! {
                biased;
                _ = async { if !*shutdown.borrow() { let _ = shutdown.changed().await; } } => {
                    if socket.send(Message::Text(r#"{"type":"shutdown"}"#.into())).await.is_ok() {
                        let _ = tokio::time::timeout(Duration::from_millis(800), async {
                            while let Some(Ok(message)) = socket.recv().await {
                                if let Message::Text(text) = message {
                                    if serde_json::from_str::<serde_json::Value>(&text).ok().is_some_and(|v| v["type"] == "shutdown_ack") { break; }
                                }
                            }
                        }).await;
                    }
                    let _ = socket.send(Message::Close(None)).await;
                    break;
                }
                Some(command) = receiver.recv() => { if socket.send(Message::Text(command.into())).await.is_err() { break; } }
                incoming = socket.recv() => match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(result) = serde_json::from_str::<PluginResult>(&text) {
                            if result.kind == "result" { bridge.deliver(&id, result); }
                        } else if let Ok(update) = serde_json::from_str::<PluginName>(&text) {
                            if update.kind == "place_name" { bridge.update_place_name(&id, update.name); }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
    bridge.end_session(&id);
}
#[derive(Deserialize)]
struct GamesResponse {
    data: Vec<GameName>,
}
#[derive(Deserialize)]
struct GameName {
    id: u64,
    name: String,
}
fn parse_experience_name(bytes: &[u8], id: u64) -> Option<String> {
    let response: GamesResponse = serde_json::from_slice(bytes).ok()?;
    let game = response.data.into_iter().find(|game| game.id == id)?;
    let name = game.name.trim();
    if name.is_empty() || name.chars().any(char::is_control) {
        return None;
    }
    Some(name.chars().take(120).collect())
}
async fn lookup_experience_name(id: u64) -> Option<String> {
    use futures_util::StreamExt;
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let response = client
        .get(format!(
            "https://games.roblox.com/v1/games?universeIds={id}"
        ))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?;
    if response.content_length().is_some_and(|n| n > 64 * 1024) {
        return None;
    }
    let mut data = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        if data.len() + chunk.len() > 64 * 1024 {
            return None;
        }
        data.extend_from_slice(&chunk);
    }
    parse_experience_name(&data, id)
}

pub async fn events_socket(
    headers: HeaderMap,
    Extension(bridge): Extension<Arc<Bridge>>,
    ws: WebSocketUpgrade,
) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if origin != format!("http://{host}") {
        return super::error(StatusCode::FORBIDDEN, "This connection is not allowed.");
    }
    ws.max_message_size(1024).max_frame_size(1024).on_upgrade(move |mut socket| async move {
        let mut updates = bridge.updates.subscribe();
        loop {
            let snapshot = updates.borrow_and_update().clone();
            if socket.send(Message::Text(serde_json::to_string(&snapshot).unwrap_or_default().into())).await.is_err() { break; }
            tokio::select! {
                changed = updates.changed() => if changed.is_err() { break; },
                incoming = socket.recv() => match incoming { Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break, _ => {} }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_reuses_private_key_and_preserves_it_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let plugin = dir.path().join("plugins");
        let bridge = Bridge::new(dir.path(), Some(plugin.clone())).unwrap();
        assert_eq!(bridge.plugin_install_state().unwrap(), "missing");
        bridge.install_plugin().unwrap();
        assert_eq!(bridge.plugin_install_state().unwrap(), "installed");
        bridge.verify_plugin().unwrap();
        let icon = dir.path().join("LocalAssets").join("StudioLine-logo.png");
        assert_eq!(fs::read(&icon).unwrap(), PLUGIN_ICON);
        fs::write(&icon, b"user file").unwrap();
        assert_eq!(bridge.plugin_install_state().unwrap(), "update_available");
        assert!(bridge.install_plugin().is_err());
        assert_eq!(fs::read(&icon).unwrap(), b"user file");
        fs::remove_file(&icon).unwrap();
        bridge.install_plugin().unwrap();
        assert_eq!(fs::read(&icon).unwrap(), PLUGIN_ICON);
        let bytes = fs::read_to_string(plugin.join("StudioLine.rbxmx")).unwrap();
        assert!(bytes.contains(&bridge.key));
        assert!(!bytes.contains(KEY_MARKER));
        assert!(bytes.contains("&gt;"));
        fs::write(plugin.join("StudioLine.rbxmx"), "previous version").unwrap();
        assert_eq!(bridge.plugin_install_state().unwrap(), "update_available");
        bridge.install_plugin().unwrap();
        assert_eq!(bridge.plugin_install_state().unwrap(), "installed");
        assert_eq!(fs::read_dir(&plugin).unwrap().count(), 1);
        let other = Bridge::new(dir.path(), Some(plugin)).unwrap();
        assert_eq!(bridge.key, other.key);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(dir.path().join("studio-plugin.key"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[tokio::test]
    #[ignore = "Calls the Roblox public experience metadata API"]
    async fn public_experience_name_lookup_works() {
        let name = lookup_experience_name(245662005).await;
        assert!(name.is_some(), "Roblox public experience lookup failed");
    }
    #[test]
    fn experience_lookup_accepts_only_matching_safe_name() {
        assert_eq!(
            parse_experience_name(br#"{"data":[{"id":1,"name":"My game"}]}"#, 1).as_deref(),
            Some("My game")
        );
        assert!(parse_experience_name(br#"{"data":[{"id":2,"name":"Wrong"}]}"#, 1).is_none());
        assert!(parse_experience_name(br#"{"data":[{"id":1,"name":"bad\nname"}]}"#, 1).is_none());
        assert!(parse_experience_name(b"not json", 1).is_none());
    }
    #[test]
    fn corrupt_key_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("studio-plugin.key"), "bad").unwrap();
        assert!(Bridge::new(dir.path(), None).is_err());
        assert_eq!(
            fs::read_to_string(dir.path().join("studio-plugin.key")).unwrap(),
            "bad"
        );
    }
    #[tokio::test]
    async fn only_one_session_and_project_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = Bridge::new(dir.path(), None).unwrap();
        let (tx, _) = mpsc::channel(1);
        assert!(bridge.start_session("Place".into(), 0, tx.clone()).is_err());
        bridge.activate("one".into()).unwrap();
        let id = bridge.start_session("Place".into(), 0, tx.clone()).unwrap();
        assert!(bridge.start_session("Other".into(), 0, tx).is_err());
        assert!(bridge.activate("two".into()).is_err());
        bridge.end_session(&id);
        assert!(!bridge.status().connected);
        assert!(bridge.activate("two".into()).is_ok());
    }
    #[tokio::test]
    async fn deleting_active_project_requires_studio_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let bridge = Bridge::new(dir.path(), None).unwrap();
        let store = super::super::Store::memory().unwrap();
        let project = store.create("Test").unwrap();
        assert!(bridge
            .activate_existing(&store, project.id.clone())
            .unwrap());
        let (tx, _) = mpsc::channel(1);
        let session = bridge.start_session("Place".into(), 0, tx).unwrap();
        assert!(bridge.delete_project(&store, &project.id).is_err());
        assert!(store.get(&project.id).unwrap().is_some());
        bridge.end_session(&session);
        assert!(bridge.delete_project(&store, &project.id).unwrap());
        assert!(bridge.status().project_id.is_none());
        assert!(!bridge.activate_existing(&store, project.id).unwrap());
    }
    #[tokio::test]
    async fn real_websocket_authentication_inspection_and_disconnect() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{
            connect_async,
            tungstenite::{client::IntoClientRequest, Message as ClientMessage},
        };
        let dir = tempfile::tempdir().unwrap();
        let bridge = Arc::new(Bridge::new(dir.path(), None).unwrap());
        bridge.activate("project".to_owned()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let store = Arc::new(crate::db::Store::memory().unwrap());
        let bridge_for_server = bridge.clone();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                crate::app(
                    store,
                    port,
                    Arc::new(tokio::sync::Notify::new()),
                    bridge_for_server,
                ),
            )
            .await
            .unwrap();
        });

        let base = format!("ws://127.0.0.1:{port}");
        let mut browser_request = format!("{base}/api/studio/socket")
            .into_client_request()
            .unwrap();
        browser_request.headers_mut().insert(
            "origin",
            format!("http://127.0.0.1:{port}").parse().unwrap(),
        );
        assert!(connect_async(browser_request).await.is_err());
        let (mut bad, _) = connect_async(format!("{base}/api/studio/socket"))
            .await
            .unwrap();
        bad.send(ClientMessage::Text(serde_json::json!({"type":"hello", "token":"wrong", "place_name":"Place", "place_id":0}).to_string().into())).await.unwrap();
        let rejected = bad.next().await.unwrap().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&rejected).unwrap()["type"],
            "error"
        );
        assert!(!bridge.status().connected);

        let mut event_request = format!("{base}/api/studio/events")
            .into_client_request()
            .unwrap();
        event_request.headers_mut().insert(
            "origin",
            format!("http://127.0.0.1:{port}").parse().unwrap(),
        );
        let (mut events, _) = connect_async(event_request).await.unwrap();
        let initial = events.next().await.unwrap().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&initial).unwrap()["connected"],
            false
        );
        let (mut plugin, _) = connect_async(format!("{base}/api/studio/socket"))
            .await
            .unwrap();
        plugin.send(ClientMessage::Text(serde_json::json!({"type":"hello", "token": bridge.key, "place_name":"Test place", "place_id":123}).to_string().into())).await.unwrap();
        let connected = plugin.next().await.unwrap().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&connected).unwrap()["type"],
            "connected"
        );
        let update = events.next().await.unwrap().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&update).unwrap()["connected"],
            true
        );
        plugin
            .send(ClientMessage::Text(
                serde_json::json!({"type":"place_name", "name":"Published name"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let renamed = events.next().await.unwrap().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&renamed).unwrap()["place_name"],
            "Published name"
        );
        let session_id = bridge
            .inner
            .lock()
            .unwrap()
            .session
            .as_ref()
            .unwrap()
            .id
            .clone();
        bridge.update_experience_name("not-this-session", "Wrong".into());
        assert!(bridge.status().experience_name.is_none());
        bridge.update_experience_name(&session_id, "Experience title".into());
        let experience_update = events.next().await.unwrap().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&experience_update).unwrap()
                ["experience_name"],
            "Experience title"
        );
        assert!(bridge.install_plugin().is_err());
        let (mut duplicate, _) = connect_async(format!("{base}/api/studio/socket"))
            .await
            .unwrap();
        duplicate.send(ClientMessage::Text(serde_json::json!({"type":"hello", "token":bridge.key, "place_name":"Other", "place_id":456}).to_string().into())).await.unwrap();
        let duplicate_response = duplicate
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&duplicate_response).unwrap()["type"],
            "error"
        );
        assert_eq!(
            bridge.status().place_name.as_deref(),
            Some("Published name")
        );

        let cloned = bridge.clone();
        let pending = tokio::spawn(async move { cloned.inspect("project").await });
        let command = plugin.next().await.unwrap().unwrap().into_text().unwrap();
        let command: serde_json::Value = serde_json::from_str(&command).unwrap();
        assert_eq!(command["command"], "inspect_place");
        plugin.send(ClientMessage::Text(serde_json::json!({"type":"result", "id":command["id"], "ok":true, "data":{"name":"Test place", "place_id":123, "workspace_children":[{"name":"Baseplate", "class_name":"Part"}], "total_children":1}}).to_string().into())).await.unwrap();
        let result = pending.await.unwrap().unwrap();
        assert_eq!(result.name, "Test place");
        assert_eq!(result.experience_name.as_deref(), Some("Experience title"));
        assert_eq!(result.workspace_children[0].class_name, "Part");

        let cloned = bridge.clone();
        let pending = tokio::spawn(async move { cloned.inspect("project").await });
        let _ = plugin.next().await.unwrap().unwrap();
        plugin.close(None).await.unwrap();
        assert!(pending.await.unwrap().is_err());
        let update = events.next().await.unwrap().unwrap().into_text().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&update).unwrap()["connected"],
            false
        );
        let (mut reconnected, _) = connect_async(format!("{base}/api/studio/socket"))
            .await
            .unwrap();
        reconnected.send(ClientMessage::Text(serde_json::json!({"type":"hello", "token":bridge.key, "place_name":"Again", "place_id":789}).to_string().into())).await.unwrap();
        let response = reconnected
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).unwrap()["type"],
            "connected"
        );
        assert_eq!(bridge.status().place_name.as_deref(), Some("Again"));
        let shutdown_bridge = bridge.clone();
        let stopping = tokio::spawn(async move { shutdown_bridge.prepare_shutdown().await });
        let message = reconnected
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&message).unwrap()["type"],
            "shutdown"
        );
        reconnected
            .send(ClientMessage::Text(r#"{"type":"shutdown_ack"}"#.into()))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), stopping)
            .await
            .unwrap()
            .unwrap();
        assert!(!bridge.status().connected);
        let (tx, _) = mpsc::channel(1);
        assert!(bridge.start_session("Late".into(), 0, tx).is_err());
        server.abort();
    }
}
