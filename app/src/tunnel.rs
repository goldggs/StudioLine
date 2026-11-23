//! Managed Cloudflare Quick Tunnel. Only the dedicated agent listener may be forwarded.
//! The desktop/admin listener must never be passed to this module.
#[cfg(any(windows, test))]
use futures_util::StreamExt;
use serde::Serialize;
#[cfg(any(windows, test, debug_assertions))]
use sha2::{Digest, Sha256};
#[cfg(any(windows, test, debug_assertions))]
use std::io::Read;
#[cfg(any(windows, test))]
use std::io::Write;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::watch,
};

#[cfg(any(windows, test))]
const RELEASE: &str = "2026.11.1";
#[cfg(any(windows, test))]
const WINDOWS_HASH: &str = "341deda3db5ec498a0482254d6c38c0b7cf01a30c37bb5dd95d2216d08955e35";
#[cfg(any(windows, test, debug_assertions))]
const MAX_BYTES: u64 = 70 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct TunnelStatus {
    pub state: &'static str,
    pub url: Option<String>,
    pub error: Option<&'static str>,
}
impl Default for TunnelStatus {
    fn default() -> Self {
        Self {
            state: "stopped",
            url: None,
            error: None,
        }
    }
}

pub struct Tunnel {
    directory: PathBuf,
    port: u16,
    status: watch::Sender<TunnelStatus>,
    stop: watch::Sender<bool>,
    starting: Mutex<bool>,
}
impl Tunnel {
    pub fn new(directory: PathBuf, agent_port: u16) -> Arc<Self> {
        let (status, _) = watch::channel(TunnelStatus::default());
        let (stop, _) = watch::channel(false);
        Arc::new(Self {
            directory,
            port: agent_port,
            status,
            stop,
            starting: Mutex::new(false),
        })
    }
    pub fn status(&self) -> TunnelStatus {
        self.status.borrow().clone()
    }
    pub fn start(self: &Arc<Self>) -> Result<(), &'static str> {
        let mut starting = self.starting.lock().map_err(|_| "Tunnel is unavailable.")?;
        if *starting {
            return if *self.stop.borrow() {
                Err("The tunnel is stopping. Try again shortly.")
            } else {
                Ok(())
            };
        }
        if self.port == 0 {
            return Err("Agent service is not available.");
        }
        *starting = true;
        self.stop.send_replace(false);
        self.status.send_replace(TunnelStatus {
            state: "starting",
            url: None,
            error: None,
        });
        let this = self.clone();
        tokio::spawn(async move {
            let result = this.run().await;
            let stopped = *this.stop.borrow();
            this.status.send_replace(if stopped {
                TunnelStatus::default()
            } else {
                TunnelStatus {
                    state: "error",
                    url: None,
                    error: Some(
                        result
                            .err()
                            .unwrap_or("The tunnel stopped. Start it again."),
                    ),
                }
            });
            if let Ok(mut busy) = this.starting.lock() {
                *busy = false;
            }
        });
        Ok(())
    }
    pub fn stop(&self) {
        self.stop.send_replace(true);
        if self.starting.lock().is_ok_and(|busy| *busy) {
            self.status.send_replace(TunnelStatus {
                state: "stopping",
                url: None,
                error: None,
            });
        }
    }
    #[cfg(test)]
    pub(crate) fn test_ready(&self) {
        self.status.send_replace(TunnelStatus {
            state: "ready",
            url: Some("https://test.trycloudflare.com".into()),
            error: None,
        });
    }
    async fn run(&self) -> Result<(), &'static str> {
        let mut stop = self.stop.subscribe();
        let component = tokio::select! {
            result = ensure_component(&self.directory) => result?,
            _ = wait_for_stop(&mut stop) => return Ok(()),
        };
        if *stop.borrow() {
            return Ok(());
        }
        // Explicit, isolated empty configuration prevents picking up the user's unrelated tunnel config.
        let config = self.directory.join("cloudflared-empty.yml");
        std::fs::write(&config, b"{}\n").map_err(|_| "Could not prepare the tunnel component.")?;
        let mut command = tokio::process::Command::new(component);
        command
            .arg("--config")
            .arg(config)
            .args(["tunnel", "--no-autoupdate", "--url"])
            .arg(format!("http://127.0.0.1:{}", self.port))
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut child = command
            .spawn()
            .map_err(|_| "Could not start the tunnel component.")?;
        let stderr = child
            .stderr
            .take()
            .ok_or("Could not start the tunnel component.")?;
        let mut lines = BufReader::new(stderr).lines();
        let deadline = tokio::time::sleep(Duration::from_secs(90));
        tokio::pin!(deadline);
        let mut url = None;
        let mut registered = false;
        let mut ready = false;
        loop {
            tokio::select! {
                _ = wait_for_stop(&mut stop) => { let _ = child.kill().await; return Ok(()); },
                _ = &mut deadline, if !ready => { let _ = child.kill().await; return Err("The tunnel did not connect in time. Check your connection and try again."); },
                exit = child.wait() => { let _ = exit; return Err("The tunnel stopped. Start it again."); },
                line = lines.next_line() => match line {
                    Ok(Some(line)) => {
                        if url.is_none() { url = parse_tunnel_url(&line); }
                        if line.contains("Connection terminated") || line.contains("no more connections active") {
                            registered = false;
                            ready = false;
                            deadline.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(90));
                            self.status.send_replace(TunnelStatus {state:"starting", url:None, error:None});
                        }
                        if line.contains("Registered tunnel connection") { registered = true; }
                        if !ready && registered && url.is_some() && !*stop.borrow() {
                            ready = true;
                            self.status.send_replace(TunnelStatus {state:"ready", url:url.clone(), error:None});
                        }
                        // Never persist cloudflared logs: they can contain tunnel details.
                    }
                    _ => { let _ = child.kill().await; return Err("The tunnel stopped unexpectedly. Try again."); }
                }
            }
        }
    }
}
async fn wait_for_stop(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow_and_update() {
            break;
        }
        if rx.changed().await.is_err() {
            break;
        }
    }
}
fn parse_tunnel_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let candidate: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '-' | '.'))
        .collect();
    let host = candidate
        .strip_prefix("https://")?
        .strip_suffix(".trycloudflare.com")?;
    if host.is_empty()
        || host.len() > 63
        || host.starts_with('-')
        || host.ends_with('-')
        || !host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return None;
    }
    Some(candidate)
}
#[cfg(any(windows, test, debug_assertions))]
fn valid_file(path: &Path, expected: &str) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    if file
        .metadata()
        .map_or(true, |m| !m.is_file() || m.len() > MAX_BYTES)
    {
        return false;
    }
    let mut reader = std::io::BufReader::new(file);
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(n) => hash.update(&bytes[..n]),
            Err(_) => return false,
        }
    }
    format!("{:x}", hash.finalize()) == expected
}
async fn ensure_component(dir: &Path) -> Result<PathBuf, &'static str> {
    #[cfg(all(not(windows), debug_assertions))]
    {
        let path = std::env::var_os("STUDIOLINE_TEST_CLOUDFLARED")
            .map(PathBuf::from)
            .ok_or("The managed tunnel component is available on Windows only.")?;
        if !valid_file(
            &path,
            "97bb3b02d23a5980fc9c15a1a270e692da1923f103f920d5b16d4931d5ab55e7",
        ) {
            return Err("The test tunnel component did not pass verification.");
        }
        std::fs::create_dir_all(dir)
            .map_err(|_| "Could not create the tunnel component folder.")?;
        Ok(path)
    }
    #[cfg(all(not(windows), not(debug_assertions)))]
    {
        let _ = dir;
        Err("The managed tunnel component is available on Windows only.")
    }
    #[cfg(windows)]
    {
        download_windows_component(dir).await
    }
}
#[cfg(any(windows, test))]
async fn download_windows_component(dir: &Path) -> Result<PathBuf, &'static str> {
    std::fs::create_dir_all(dir).map_err(|_| "Could not create the tunnel component folder.")?;
    let path = dir.join("cloudflared.exe");
    if valid_file(&path, WINDOWS_HASH) {
        return Ok(path);
    }
    let client = reqwest::Client::builder()
        .https_only(true)
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|_| "Could not prepare the tunnel download.")?;
    let url = format!("https://github.com/cloudflare/cloudflared/releases/download/{RELEASE}/cloudflared-windows-amd64.exe");
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| {
            "Could not download the tunnel component. Check your connection and try again."
        })?
        .error_for_status()
        .map_err(|_| "Could not download the tunnel component. Try again later.")?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BYTES)
    {
        return Err("Tunnel component download was too large.");
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(dir).map_err(|_| "Could not save the tunnel component.")?;
    let mut stream = response.bytes_stream();
    let mut hash = Sha256::new();
    let mut size = 0u64;
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|_| "Tunnel download was interrupted. Try again.")?;
        size = size
            .checked_add(bytes.len() as u64)
            .ok_or("Tunnel component download was too large.")?;
        if size > MAX_BYTES {
            return Err("Tunnel component download was too large.");
        }
        hash.update(&bytes);
        temp.write_all(&bytes)
            .map_err(|_| "Could not save the tunnel component. Check disk space.")?;
    }
    if format!("{:x}", hash.finalize()) != WINDOWS_HASH {
        return Err("The tunnel component did not pass verification. It was not installed.");
    }
    temp.as_file()
        .sync_all()
        .map_err(|_| "Could not save the tunnel component.")?;
    temp.persist(&path)
        .map_err(|_| "Could not replace the tunnel component. Close StudioLine and try again.")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Downloads the official 55 MB Windows tunnel component over the internet"]
    async fn official_windows_download_matches_pinned_hash_and_is_reused() {
        let directory = tempfile::tempdir().unwrap();
        let path = download_windows_component(directory.path()).await.unwrap();
        assert!(valid_file(&path, WINDOWS_HASH));
        assert_eq!(
            download_windows_component(directory.path()).await.unwrap(),
            path
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    #[test]
    fn validates_urls_strictly() {
        assert_eq!(
            parse_tunnel_url("INF | https://one-two.trycloudflare.com |"),
            Some("https://one-two.trycloudflare.com".into())
        );
        for bad in [
            "https://bad.trycloudflare.com.evil.test",
            "http://abc.trycloudflare.com",
            "https://a_b.trycloudflare.com",
            "https://-bad.trycloudflare.com",
            "https://bad.trycloudflare.com/path",
        ] {
            assert_eq!(parse_tunnel_url(bad), None);
        }
    }
    #[test]
    fn hashes_component_before_trust() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("component");
        std::fs::write(&p, b"abc").unwrap();
        assert!(valid_file(
            &p,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
        assert!(!valid_file(&p, WINDOWS_HASH));
        std::fs::write(p, b"bad").unwrap();
        assert!(!valid_file(
            dir.path().join("component").as_path(),
            WINDOWS_HASH
        ));
    }
}
