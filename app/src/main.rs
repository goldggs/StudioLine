#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use directories::ProjectDirs;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Notify;

#[cfg(windows)]
mod uninstall;

const PORT: u16 = 38741;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        tracing::error!("Startup failed: {message}");
        show_error(&message);
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    #[cfg(windows)]
    if std::env::args_os().any(|arg| arg == "--finish-uninstall") {
        return uninstall::finish()
            .map_err(|e| format!("StudioLine uninstall cleanup did not complete.\n\n{e}"));
    }
    #[cfg(windows)]
    if std::env::args_os().any(|arg| arg == "--uninstall") {
        return uninstall::run().map_err(|e| format!("Uninstall did not complete. Some StudioLine files may already have been removed. Fix the problem and retry.\n\n{e}"));
    }
    #[cfg(windows)]
    let _uninstall_guard = if std::env::var_os("STUDIOLINE_DATA_DIR").is_none() {
        Some(uninstall::startup_guard()?)
    } else {
        None
    };
    let data = data_directory()?;
    fs::create_dir_all(&data).map_err(|_| {
        "Could not create the local data folder. Check that you have permission to write to it."
            .to_owned()
    })?;
    let logs = data.join("logs");
    fs::create_dir_all(&logs).map_err(|_| {
        "Could not create the log folder. Check folder permissions and try again.".to_owned()
    })?;
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join("app.log"))
        .map_err(|_| {
            "Could not open the local log file. Check folder permissions and try again.".to_owned()
        })?;
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(std::sync::Mutex::new(log))
        .init();
    if std::env::args().any(|arg| arg == "--verify-install") {
        if !studioline::interface_is_embedded() {
            return Err(
                "The StudioLine interface is missing. Reinstall the app and try again.".to_owned(),
            );
        }
        let _store = open_store(&data)?;
        tracing::info!("Installation check passed");
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--install-plugin") {
        let bridge = open_bridge(&data)?;
        bridge.install_plugin().map_err(str::to_owned)?;
        return Ok(());
    }
    if std::env::args().any(|arg| arg == "--verify-plugin") {
        let bridge = open_bridge(&data)?;
        bridge.verify_plugin().map_err(str::to_owned)?;
        return Ok(());
    }
    let instance_lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(data.join("instance.lock"))
        .map_err(|_| {
            "Could not open the application lock file. Check folder permissions and try again."
                .to_owned()
        })?;
    let should_open = !std::env::args().any(|arg| arg == "--no-open");
    match fs2::FileExt::try_lock_exclusive(&instance_lock) {
        Ok(()) => run_primary(data, instance_lock, should_open).await,
        Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            for _ in 0..30 {
                if existing_instance_ready() {
                    if should_open { open_or_report()?; }
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err("StudioLine is already starting or running, but its local service did not respond. Wait a moment and try again.".to_owned())
        }
        Err(_) => Err("Could not check whether StudioLine is already running. Check folder permissions and try again.".to_owned()),
    }
}

async fn run_primary(data: PathBuf, _instance_lock: File, should_open: bool) -> Result<(), String> {
    #[cfg(windows)]
    if std::env::var_os("STUDIOLINE_DATA_DIR").is_none() {
        if let Err(error) = uninstall::register() {
            tracing::error!("Could not register Windows uninstall: {error}");
            show_error("StudioLine started, but could not register in Windows Installed Apps. Check folder and registry permissions, then restart StudioLine.");
        }
    }
    let addr: SocketAddr = ([127, 0, 0, 1], PORT).into();
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        tracing::error!("Could not bind local port: {e}");
        "StudioLine could not start because its local port is in use by another application. Close that application and try again.".to_owned()
    })?;
    let store = open_store(&data)?;
    let bridge = open_bridge(&data)?;
    // Only this separate, narrow service is ever forwarded by cloudflared.
    let agent_listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|_| "Could not start the local agent service.".to_owned())?;
    let agent_port = agent_listener
        .local_addr()
        .map_err(|_| "Could not start the local agent service.".to_owned())?
        .port();
    let tunnel = studioline::tunnel::Tunnel::new(data.join("components"), agent_port);
    let agents = studioline::agent::Agents::new(bridge.clone(), tunnel);
    let agent_router = studioline::agent::public_router(agents.clone());
    let agent_server_state = agents.clone();
    tokio::spawn(async move {
        if axum::serve(agent_listener, agent_router).await.is_err() {
            tracing::error!("Agent service stopped unexpectedly");
        }
        agent_server_state.shutdown();
    });
    let shutdown = Arc::new(Notify::new());
    tracing::info!("Local app started");
    if should_open {
        if let Err(message) = open_or_report() {
            show_error(&message);
        }
    }
    let app = studioline::app_with_agents(
        store,
        PORT,
        shutdown.clone(),
        bridge.clone(),
        Some(agents.clone()),
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = shutdown.notified() => {} }
            agents.shutdown();
            bridge.prepare_shutdown().await;
        })
        .await
        .map_err(|e| {
            tracing::error!("Local service stopped: {e}");
            "The local service stopped unexpectedly. Check the local log and restart StudioLine."
                .to_owned()
        })?;
    tracing::info!("Local app stopped");
    Ok(())
}
fn open_store(data: &Path) -> Result<studioline::AppState, String> {
    studioline::open_store(&data.join("projects.sqlite3")).map_err(|e| {
        tracing::error!("Could not open project database: {e}");
        "StudioLine could not open its project database. Your project data was not reset. Check the local log and try again.".to_owned()
    })
}
fn open_bridge(data: &Path) -> Result<Arc<studioline::studio::Bridge>, String> {
    #[cfg(windows)]
    let plugin_dir = std::env::var_os("LOCALAPPDATA")
        .map(|root| PathBuf::from(root).join("Roblox").join("Plugins"));
    #[cfg(not(windows))]
    let plugin_dir = None;
    studioline::studio::Bridge::new(data, plugin_dir).map(Arc::new).map_err(|_| "StudioLine could not read its local plugin key. Your data was not reset. Check local folder permissions or reinstall the plugin.".to_owned())
}
fn data_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("STUDIOLINE_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    ProjectDirs::from("", "StudioLine", "StudioLine")
        .map(|dirs| dirs.data_local_dir().to_path_buf())
        .ok_or_else(|| "Could not find a local data folder for StudioLine.".to_owned())
}
fn existing_instance_ready() -> bool {
    let addr: SocketAddr = ([127, 0, 0, 1], PORT).into();
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(300)) else {
        return false;
    };
    if stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .is_err()
        || stream
            .set_write_timeout(Some(Duration::from_millis(300)))
            .is_err()
    {
        return false;
    }
    let request =
        format!("GET /api/health HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = Vec::new();
    if stream.take(4096).read_to_end(&mut response).is_err() {
        return false;
    }
    let Ok(text) = std::str::from_utf8(&response) else {
        return false;
    };
    if !text.starts_with("HTTP/1.1 200 ") {
        return false;
    }
    let Some((_, body)) = text.split_once("\r\n\r\n") else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .is_some_and(|value| value["app"] == "StudioLine" && value["status"] == "ready")
}
fn open_or_report() -> Result<(), String> {
    let url = format!("http://127.0.0.1:{PORT}/");
    open_browser(&url).map_err(|e| {
        tracing::error!("Could not open browser: {e}");
        format!("StudioLine is running, but could not open your browser. Open {url} manually.")
    })
}
#[cfg(windows)]
fn open_browser(url: &str) -> std::io::Result<()> {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            url.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    } as isize;
    if result > 32 {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "Windows could not open the default browser (code {result})"
        )))
    }
}
#[cfg(not(windows))]
fn open_browser(url: &str) -> std::io::Result<()> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .map(|_| ())
}
#[cfg(windows)]
fn show_error(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "StudioLine".encode_utf16().chain(Some(0)).collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
#[cfg(not(windows))]
fn show_error(message: &str) {
    eprintln!("{message}");
}
