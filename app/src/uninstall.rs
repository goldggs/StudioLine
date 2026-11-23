//! Per-user uninstall for the standalone app. Only explicitly named, owned files are removed.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Read},
    os::windows::{ffi::OsStrExt, fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::ERROR_SUCCESS,
    System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_WRITE, REG_DWORD, REG_SZ,
    },
    UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_YESNO,
    },
};

const KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\StudioLine";
const REPARSE: u32 = 0x400;
#[derive(Serialize, Deserialize)]
struct Manifest {
    source: PathBuf,
    sha256: String,
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}
fn w(s: &str) -> Vec<u16> {
    wide(OsStr::new(s))
}
fn local() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| "Could not find the Windows local application data folder.".into())
}
fn root() -> Result<PathBuf, String> {
    Ok(local()?.join("StudioLine").join("StudioLine"))
}
fn owned_dir(path: &Path) -> Result<(), String> {
    // Do not follow junctions or symlinks in managed paths, even if they were added later.
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(m) if m.file_attributes() & REPARSE != 0 => {
                return Err(format!(
                    "A managed folder is redirected: {}",
                    part.display()
                ))
            }
            Ok(m) if !m.is_dir() => return Err(format!("Not a folder: {}", part.display())),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("Cannot inspect {}: {e}", part.display())),
        }
    }
    Ok(())
}
// A separate lock protects startup before any data or logs are opened. It also protects
// the uninstaller; the older data lock is checked as well for previously running builds.
fn gate(exclusive: bool) -> Result<File, String> {
    let root = root()?;
    owned_dir(&root)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let path = root.join("operation.lock");
    if let Ok(m) = fs::symlink_metadata(&path) {
        if m.file_attributes() & REPARSE != 0 || !m.is_file() {
            return Err("StudioLine's operation lock is redirected or invalid.".into());
        }
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| e.to_string())?;
    let result = if exclusive {
        fs2::FileExt::try_lock_exclusive(&file)
    } else {
        fs2::FileExt::try_lock_shared(&file)
    };
    result.map_err(|_| if exclusive { "StudioLine is still running. Shut it down from its browser window, then try uninstalling again." } else { "StudioLine is being uninstalled. Try again after uninstall finishes." }.to_owned())?;
    Ok(file)
}
pub fn startup_guard() -> Result<File, String> {
    let file = gate(false)?;
    if root()?.join("uninstall").join("cleanup.pending").exists() {
        return Err("StudioLine is finishing uninstall. Wait a minute and try again. If it does not finish, delete the remaining uninstall folder in the StudioLine local app-data folder manually.".into());
    }
    Ok(file)
}

fn hash(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
fn set_value(key: HKEY, name: &str, kind: u32, data: &[u8]) -> Result<(), String> {
    let code = unsafe {
        RegSetValueExW(
            key,
            w(name).as_ptr(),
            0,
            kind,
            data.as_ptr(),
            data.len() as u32,
        )
    };
    if code != ERROR_SUCCESS {
        return Err(format!("Windows registry error {code}"));
    }
    Ok(())
}
fn set_string(key: HKEY, name: &str, value: &OsStr) -> Result<(), String> {
    let value = wide(value);
    let bytes = unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
    set_value(key, name, REG_SZ, bytes)
}

pub fn register() -> Result<(), String> {
    let root = root()?;
    let dir = root.join("uninstall");
    owned_dir(&dir)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let source = std::env::current_exe().map_err(|e| e.to_string())?;
    let helper = dir.join("StudioLine-Uninstall.exe");
    if source == helper {
        return Err("Cannot register the uninstall helper as the application.".into());
    }
    // Atomic replacement leaves the previous copy usable if copying fails.
    let mut temp = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    io::copy(
        &mut File::open(&source).map_err(|e| e.to_string())?,
        &mut temp,
    )
    .map_err(|e| e.to_string())?;
    temp.persist(&helper).map_err(|e| e.to_string())?;
    let manifest = Manifest {
        sha256: hash(&source).map_err(|e| e.to_string())?,
        source,
    };
    let mut temp = tempfile::NamedTempFile::new_in(&dir).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut temp, &manifest).map_err(|e| e.to_string())?;
    temp.persist(dir.join("installation.json"))
        .map_err(|e| e.to_string())?;
    write_registration(&helper)
}

fn write_registration(helper: &Path) -> Result<(), String> {
    let mut key: HKEY = std::ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w(KEY).as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_WRITE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!("Windows registry error {status}"));
    }
    let result = (|| {
        set_string(key, "DisplayName", OsStr::new("StudioLine"))?;
        set_string(key, "Publisher", OsStr::new("StudioLine"))?;
        set_string(key, "DisplayVersion", OsStr::new(env!("CARGO_PKG_VERSION")))?;
        set_string(key, "DisplayIcon", helper.as_os_str())?;
        let mut command = std::ffi::OsString::from("\"");
        command.push(helper);
        command.push("\" --uninstall");
        set_string(key, "UninstallString", &command)?;
        set_value(key, "NoModify", REG_DWORD, &1u32.to_le_bytes())?;
        set_value(key, "NoRepair", REG_DWORD, &1u32.to_le_bytes())?;
        Ok(())
    })();
    unsafe {
        RegCloseKey(key);
    }
    result
}
fn dialog(message: &str, flags: u32) -> i32 {
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            w(message).as_ptr(),
            w("StudioLine uninstall").as_ptr(),
            flags,
        )
    }
}
fn remove_file(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_attributes() & REPARSE != 0 || !m.is_file() => Err(format!(
            "Refusing to remove redirected or non-file path: {}",
            path.display()
        )),
        Ok(_) => {
            fs::remove_file(path).map_err(|e| format!("Could not remove {}: {e}", path.display()))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Cannot inspect {}: {e}", path.display())),
    }
}
fn remove_known(dir: &Path, files: &[&str]) -> Result<(), String> {
    owned_dir(dir)?;
    for file in files {
        remove_file(&dir.join(file))?;
    }
    // Only empty directories are removed. Unknown files, including exports, are preserved.
    match fs::remove_dir(dir) {
        Ok(()) => {}
        Err(e)
            if e.kind() == io::ErrorKind::NotFound
                || e.kind() == io::ErrorKind::DirectoryNotEmpty => {}
        Err(e) => return Err(format!("Could not remove folder {}: {e}", dir.display())),
    }
    Ok(())
}
fn acquire_lock(data: &Path) -> Result<Option<File>, String> {
    owned_dir(data)?;
    if !data.exists() {
        return Ok(None);
    }
    let lockpath = data.join("instance.lock");
    if let Ok(m) = fs::symlink_metadata(&lockpath) {
        if m.file_attributes() & REPARSE != 0 || !m.is_file() {
            return Err("The application lock is redirected or invalid.".into());
        }
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lockpath)
        .map_err(|e| e.to_string())?;
    fs2::FileExt::try_lock_exclusive(&lock).map_err(|_| "StudioLine is still running. Shut it down from its browser window, then try uninstalling again.".to_owned())?;
    Ok(Some(lock))
}
fn remove_plugin(data: &Path, plugin: &Path) -> Result<(), String> {
    owned_dir(plugin.parent().ok_or("Invalid plugin path")?)?;
    let meta = match fs::symlink_metadata(plugin) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    if meta.file_attributes() & REPARSE != 0 || !meta.is_file() || meta.len() > 4_000_000 {
        return Err("The StudioLine plugin file is invalid or redirected. Remove it manually before retrying.".into());
    }
    let key = fs::read_to_string(data.join("studio-plugin.key")).map_err(|_| {
        "Could not verify the installed StudioLine plugin. Remove it manually before retrying."
    })?;
    let content = fs::read_to_string(plugin).map_err(|e| e.to_string())?;
    if key.len() != 64
        || !key.chars().all(|c| c.is_ascii_hexdigit())
        || !content.contains(&key)
        || !content.contains("StudioLine local plugin")
    {
        return Err("The plugin file does not match this StudioLine installation. It was preserved; inspect and remove it manually before retrying.".into());
    }
    remove_file(plugin)
}
fn remove_plugin_icon(local: &Path) -> Result<(), String> {
    let dir = local.join("Roblox").join("LocalAssets");
    owned_dir(&dir)?;
    let path = dir.join("StudioLine-logo.png");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("Cannot inspect plugin icon: {e}")),
    };
    // Preserve unrelated or redirected files. Remove only the exact logo shipped by this app.
    if metadata.file_attributes() & REPARSE != 0
        || !metadata.is_file()
        || metadata.len() > 1024 * 1024
    {
        return Ok(());
    }
    let bytes = fs::read(&path).map_err(|e| format!("Cannot read plugin icon: {e}"))?;
    if bytes == include_bytes!("../../design/StudioLine-logo.png") {
        remove_file(&path)?;
    }
    Ok(())
}
fn check_source(manifest: &Manifest, helper: &Path) -> Result<bool, String> {
    let path = &manifest.source;
    if !path.is_absolute() || path == helper {
        return Err("The saved application path is invalid.".into());
    }
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    if m.file_attributes() & REPARSE != 0 || !m.is_file() {
        return Err(format!(
            "The original application is redirected or changed; it was preserved: {}",
            path.display()
        ));
    }
    if hash(path).map_err(|e| e.to_string())? != manifest.sha256 {
        return Err(format!("The original application changed since registration and was preserved: {}. Remove it manually, then retry.", path.display()));
    }
    Ok(true)
}
pub fn run() -> Result<(), String> {
    if std::env::var_os("STUDIOLINE_DATA_DIR").is_some() {
        return Err("Uninstall is disabled while a custom test data directory is set.".into());
    }
    let root = root()?;
    let data = root.join("data");
    let dir = root.join("uninstall");
    owned_dir(&dir)?;
    let helper = dir.join("StudioLine-Uninstall.exe");
    if fs::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        != fs::canonicalize(&helper).map_err(|e| e.to_string())?
    {
        return Err(
            "Run uninstall from Windows Installed Apps after opening StudioLine at least once."
                .into(),
        );
    }
    let manifest: Manifest = serde_json::from_reader(
        File::open(dir.join("installation.json"))
            .map_err(|e| format!("Cannot read installation details: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let text = format!("Remove StudioLine?\n\nThis permanently deletes StudioLine projects, database, settings, credentials, logs, managed tunnel component and StudioLine's Roblox plugin. It also deletes this application file if unchanged:\n{}\n\nExport a database backup first if needed. Shut down StudioLine and close Roblox Studio before continuing. Roblox Studio, your Roblox place files, exported backups and independently installed Cloudflare software are not removed.\n\nThis cannot be undone.", manifest.source.display());
    if dialog(&text, MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES {
        return Ok(());
    }
    let _gate = gate(true)?;
    let lock = acquire_lock(&data)?;
    check_source(&manifest, &helper)?;
    let plugin = local()?
        .join("Roblox")
        .join("Plugins")
        .join("StudioLine.rbxmx");
    remove_plugin(&data, &plugin)?;
    remove_plugin_icon(&local()?)?;
    remove_known(
        &data.join("components"),
        &["cloudflared.exe", "cloudflared-empty.yml"],
    )?;
    remove_known(&data.join("logs"), &["app.log"])?;
    for name in [
        "projects.sqlite3",
        "projects.sqlite3-wal",
        "projects.sqlite3-shm",
        "projects.sqlite3-journal",
        "studio-plugin.key",
    ] {
        remove_file(&data.join(name))?;
    }
    if check_source(&manifest, &helper)? {
        remove_file(&manifest.source)?;
    }
    drop(lock);
    remove_known(&data, &["instance.lock"])?;
    // The registered executable is mapped while running. A temporary copy waits for
    // this process to release the gate, then removes it with normal filesystem APIs.
    let marker = dir.join("cleanup.pending");
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
        .map_err(|e| format!("Could not prepare final cleanup: {e}"))?;
    let finalizer = (|| -> Result<(), String> {
        let temp = tempfile::Builder::new()
            .prefix("StudioLine-finalize-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let executable = temp.path().join("finalize.exe");
        fs::copy(&helper, &executable).map_err(|e| e.to_string())?;
        Command::new(&executable)
            .arg("--finish-uninstall")
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
        let _ = temp.keep();
        Ok(())
    })();
    if let Err(error) = finalizer {
        let _ = remove_file(&marker);
        return Err(format!(
            "Could not start final cleanup: {error}. Retry from Installed Apps."
        ));
    }
    Ok(())
}

pub fn finish() -> Result<(), String> {
    let root = root()?;
    let dir = root.join("uninstall");
    owned_dir(&dir)?;
    let marker = dir.join("cleanup.pending");
    if !marker.is_file() {
        return Err("No uninstall is pending.".into());
    }
    let mut guard = None;
    for _ in 0..100 {
        match gate(true) {
            Ok(lock) => {
                guard = Some(lock);
                break;
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
    let Some(_guard) = guard else {
        let _ = remove_file(&marker);
        return Err(
            "StudioLine did not close in time. Shut it down and retry from Installed Apps.".into(),
        );
    };
    let helper = dir.join("StudioLine-Uninstall.exe");
    // Delete the registry entry first. If removal of the helper fails, restore it.
    let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, w(KEY).as_ptr()) };
    if code != ERROR_SUCCESS && code != 2 {
        let _ = remove_file(&marker);
        return Err(format!(
            "Could not remove the Installed Apps entry (error {code})."
        ));
    }
    if let Err(e) = remove_file(&helper) {
        let restored = write_registration(&helper);
        let _ = remove_file(&marker);
        return Err(format!(
            "{e} {}",
            if restored.is_ok() {
                "Retry from Installed Apps."
            } else {
                "The Installed Apps entry could not be restored. Remove the uninstaller manually."
            }
        ));
    }
    remove_file(&dir.join("installation.json"))?;
    remove_file(&marker)?;
    let _ = fs::remove_dir(&dir); // Only succeeds if empty.
    dialog("StudioLine has been removed. Unrecognized files and exported backups were preserved. A temporary cleanup file will be removed after this message closes.", MB_OK | MB_ICONINFORMATION);
    // The only self-deletion here concerns the temporary copy, never user data.
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let temp = std::env::temp_dir();
    let parent = me.parent().ok_or("Invalid temporary cleanup path")?;
    if !parent.starts_with(&temp)
        || !parent
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("StudioLine-finalize-"))
    {
        return Err("Temporary cleanup path was not recognized; remove it manually.".into());
    }
    let cmd = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or("Windows system folder unavailable")?
        .join("System32")
        .join("cmd.exe");
    let script = "ping -n 3 127.0.0.1 >nul & del /f /q \"%STUDIOLINE_TEMP_FINALIZER%\" 2>nul & rd \"%STUDIOLINE_TEMP_FOLDER%\" 2>nul";
    Command::new(cmd)
        .args(["/d", "/q", "/v:off", "/c"])
        .raw_arg(script)
        .env("STUDIOLINE_TEMP_FINALIZER", &me)
        .env("STUDIOLINE_TEMP_FOLDER", parent)
        .creation_flags(0x08000000)
        .spawn()
        .map_err(|e| {
            format!(
                "Could not remove temporary cleanup file {}: {e}",
                me.display()
            )
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn removes_only_named_files_and_preserves_other_contents() {
        let d = tempdir().unwrap();
        let folder = d.path().join("logs");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("app.log"), "log").unwrap();
        fs::write(folder.join("StudioLine-export.db"), "backup").unwrap();
        fs::create_dir(folder.join("extra")).unwrap();
        remove_known(&folder, &["app.log"]).unwrap();
        assert!(!folder.join("app.log").exists());
        assert_eq!(
            fs::read(folder.join("StudioLine-export.db")).unwrap(),
            b"backup"
        );
        assert!(folder.join("extra").is_dir());
    }

    #[test]
    fn source_must_be_the_unchanged_registered_file() {
        let d = tempdir().unwrap();
        let source = d.path().join("app.exe");
        fs::write(&source, b"original").unwrap();
        let m = Manifest {
            source: source.clone(),
            sha256: hash(&source).unwrap(),
        };
        assert!(check_source(&m, &d.path().join("helper.exe")).unwrap());
        fs::write(&source, b"different").unwrap();
        assert!(check_source(&m, &d.path().join("helper.exe")).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"different");
        assert!(check_source(&m, &source).is_err());
        fs::remove_file(&source).unwrap();
        assert!(!check_source(&m, &d.path().join("helper.exe")).unwrap());
    }

    #[test]
    fn plugin_requires_installation_key_and_marker() {
        let d = tempdir().unwrap();
        let data = d.path().join("data");
        let plugins = d.path().join("plugins");
        fs::create_dir(&data).unwrap();
        fs::create_dir(&plugins).unwrap();
        let plugin = plugins.join("StudioLine.rbxmx");
        let key = "a".repeat(64);
        fs::write(data.join("studio-plugin.key"), &key).unwrap();
        fs::write(&plugin, "unrelated file").unwrap();
        assert!(remove_plugin(&data, &plugin).is_err());
        assert!(plugin.exists());
        fs::write(&plugin, format!("StudioLine local plugin {key}")).unwrap();
        remove_plugin(&data, &plugin).unwrap();
        assert!(!plugin.exists());
        assert!(remove_plugin(&data, &plugin).is_ok());
    }

    #[test]
    fn plugin_icon_cleanup_preserves_unknown_files() {
        let d = tempdir().unwrap();
        let dir = d.path().join("Roblox").join("LocalAssets");
        fs::create_dir_all(&dir).unwrap();
        let icon = dir.join("StudioLine-logo.png");
        fs::write(&icon, b"user image").unwrap();
        remove_plugin_icon(d.path()).unwrap();
        assert_eq!(fs::read(&icon).unwrap(), b"user image");
        fs::write(&icon, include_bytes!("../../design/StudioLine-logo.png")).unwrap();
        remove_plugin_icon(d.path()).unwrap();
        assert!(!icon.exists());
    }

    #[test]
    fn running_instance_lock_blocks_cleanup() {
        let d = tempdir().unwrap();
        let lock = acquire_lock(d.path()).unwrap().unwrap();
        assert!(acquire_lock(d.path()).is_err());
        drop(lock);
        assert!(acquire_lock(d.path()).is_ok());
    }

    #[test]
    fn refuses_directory_in_place_of_owned_file() {
        let d = tempdir().unwrap();
        let path = d.path().join("app.log");
        fs::create_dir(&path).unwrap();
        assert!(remove_file(&path).is_err());
        assert!(path.is_dir());
    }
}
