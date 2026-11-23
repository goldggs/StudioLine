# StudioLine

<img src="ui/public/studioline-logo.svg" alt="StudioLine logo" width="96" height="96">

StudioLine is an early local workspace for collaborating with AI agents on a Roblox Studio project.

## Current status

The local Rust app serves a browser interface, stores projects in SQLite, and supports a local Studio plugin connection with read-only place inspection. The owner has tested the earlier connection build in Roblox Studio on Windows. Subsequent plugin shutdown, project deletion, and data export changes are not yet tested in real Studio or on Windows.

An experimental agent API and managed Quick Tunnel are implemented. The owner reports successfully working with a separate Arena agent on Windows. Registered Windows uninstall is implemented but requires real Windows validation. Chat, permissions, and versioning are not implemented. The app is unsigned and may trigger Windows security warnings. The Windows CI workflow has not been run.

## Intended flow

1. Open the StudioLine executable. The planned integrated setup will handle required components.
2. Open the main app. It starts the local service and opens the workspace in a browser tab.
3. Create or open a project, then connect the Studio plugin.
4. Give separate Arena chats their individual connection prompts and collaborate in the workspace.

## Architecture

- **Local app:** Rust and Axum.
- **Interface:** React and TypeScript, compiled into static files and embedded in the app.
- **Storage:** SQLite, managed by the app. No separate database server.
- **Studio plugin:** Luau, connected to the app over a local authenticated WebSocket.
- **Agent access:** a separate agent-only HTTP service behind an authenticated Cloudflare Quick Tunnel. The local administrative API is not tunneled.
- **Platform:** Windows only for the initial release. Linux is currently used for development tests.

Quick Tunnels are temporary, may change address on restart, and have no uptime guarantee. Their suitability beyond early development needs evaluation.

## Developer setup in a Unix-like sandbox

These scripts are for development, not end-user installation. They require Bash, `curl`, Node.js, npm, `unzip`, and a C compiler. The setup script downloads Rust, npm dependencies, and a pinned Luau CLI for Linux x64; inspect it before running.

```sh
./scripts/setup-toolchain.sh
./scripts/build.sh
./scripts/test.sh
source scripts/toolchain-env.sh
"$CARGO_TARGET_DIR/debug/studioline" --no-open
```

Open `http://127.0.0.1:38741` in a browser. Omit `--no-open` to ask the app to open the browser. Use the Shut down button in the interface or press Ctrl+C in development mode to stop it.

By default, toolchains, dependency caches, and build outputs live in `/var/tmp/studioline-toolchain`, outside the repository. Set `STUDIOLINE_TOOLS_DIR` to change that location. `ui/node_modules` is an ignored symlink into the temporary directory. Temporary files may disappear between sessions; run setup again if needed. The UI build is generated in `ui/dist` and ignored by Git. It must be built before compiling Rust because the executable embeds it; `scripts/build.sh` handles this.

Project data and logs use the platform's local app-data directory. For isolated tests, set `STUDIOLINE_DATA_DIR` to a writable temporary directory. Do not expose the local app directly to the public internet.

## Repository layout

- `app/`: Rust app and local API.
- `ui/`: browser interface.
- `plugin/`: Luau Studio plugin and simulated tests.
- `scripts/`: temporary developer toolchain setup, build, and test scripts.
- `tests/`: test guidance and cross-component plans. Browser tests live in `ui/e2e/`; plugin mock tests live in `plugin/tests/`.
- `docs/`: design philosophy and architecture notes.

## Studio connection test

In the Windows build, open a project and click **Install Studio plugin**, or **Update Studio plugin** if an earlier copy is present. If the current plugin is already installed, the button is hidden. Restart Studio, open a game, open the StudioLine panel from the Plugins toolbar, and click **Connect**. In the app, **Inspect game** requests read-only information about the open place. The app attempts to look up the published experience name from Roblox using its experience ID; if unavailable it falls back to the published place name, then Studio's local name. The lookup is asynchronous and may not work for unpublished or inaccessible experiences. The plugin toolbar logo uses a local Roblox asset installed by StudioLine; this integration still needs verification in real Roblox Studio, especially if its local-assets folder is customized. Only one Studio session can be connected at a time. Disconnect the plugin before opening a different project or deleting the connected project. On a deliberate app shutdown, the connected plugin is notified; on an unexpected failure, it stops after a few retries.

The installed plugin contains a per-installation connection key. Do not share the installed `.rbxmx` file or `studio-plugin.key`. The source in this repository contains only a placeholder. Studio may ask for network permission. The Studio connection is local; agent access uses an isolated, read-only API through a temporary tunnel. Export downloads a consistent SQLite snapshot of StudioLine's data, not the Roblox place; save that separately in Studio.

## Windows uninstall

Running the standalone executable normally registers StudioLine for the current Windows user in Settings > Apps > Installed apps. No installer or administrator access is required. Installed Apps uses a private uninstaller copy, so it remains accessible if the downloaded executable is moved. Before uninstalling, export any needed SQLite backup, save Roblox places separately, shut down StudioLine from its browser, and close Studio. The uninstaller asks for confirmation before deleting StudioLine data, its managed plugin and tunnel, and the registered original executable if unchanged. It preserves unknown files and exported backups, Roblox Studio, Roblox places, and independently installed Cloudflare software. If the original executable was changed, or the plugin cannot be verified, removal stops and asks you to inspect it manually. An error can occur after some files have already been deleted; fix it and retry from Installed Apps. A temporary cleanup copy removes the running uninstaller; empty folders, a small lock file, or a temporary cleanup copy may remain if final self-cleanup fails. If the original download was moved or copied, additional copies must be removed manually. Custom `STUDIOLINE_DATA_DIR` test runs do not register or run uninstall.

Windows uninstall has been exercised under Wine, including cancellation, a running-app check, file preservation and removal, but has not yet been tested on a real Windows machine. Use a disposable Windows test account and keep backups before testing.

## Agent connection test

In a project, choose **Connect agents**, then **Start agent connection**. On Windows, StudioLine downloads a pinned cloudflared release and verifies its SHA-256 before use. No separate download or Cloudflare account is required. The tunnel forwards only the agent API, never the local admin interface. **Create agent prompt** issues one credential; its address and prompt are masked until revealed. Copy the prompt into a genuinely separate Arena agent chat. The agent chooses its name and role, registers, and can request a read-only inspection. No editing, chat, or autonomous execution is included in this slice.

Credentials are held in memory and revoked when the tunnel stops or StudioLine closes. Stopping and restarting a tunnel changes the address; issue new prompts. Don't share credentials in screenshots, logs, or public chats. A Quick Tunnel is temporary and has no uptime guarantee.

This remains an experimental connection: the owner reports successful Windows testing with a separate Arena agent; Linux development tests also exercised an actual Cloudflare tunnel with a simulated Studio plugin. Do not rely on it for sensitive or production work yet.

## License

A license has not yet been selected. Do not assume permission to reuse or redistribute the code until one is added.
