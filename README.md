# StudioLine

StudioLine is a planned local workspace for collaborating with AI agents on a Roblox Studio project. This repository currently contains the initial project structure and design notes, not a working application.

## Intended flow

1. Install StudioLine using the Windows setup application.
2. Open the main app. It starts the local service and opens the workspace in a browser tab.
3. Create or open a project, then connect the Studio plugin.
4. Give separate Arena chats their individual connection prompts and collaborate in the workspace.

## Planned architecture

- **Local app:** Rust, with Axum for the local API and WebSocket server.
- **Interface:** React and TypeScript, served locally by the app.
- **Storage:** SQLite, managed by the app. No separate database server.
- **Studio plugin:** Luau, connecting to the local app over WebSocket.
- **Agent access:** an authenticated agent API reachable through a Cloudflare Quick Tunnel.
- **Platform:** Windows only for the initial release.

Quick Tunnels are temporary, may change address on restart, and have no uptime guarantee. Their suitability beyond early development needs evaluation.

## Repository layout

- `app/` contains the planned Rust application and local API.
- `ui/` contains the planned browser interface.
- `plugin/` contains the planned Roblox Studio plugin.
- `setup/` contains the planned Windows installer and setup checks.
- `tests/` contains cross-component and adversarial test plans and suites.
- `docs/` contains the design philosophy and architecture notes.

These directories are placeholders. Setup, implementation, and build instructions will be added when they exist.

## Status

This is an early scaffold. Agent connections, Studio commands, installation, and versioning are not implemented yet. The standalone UI mockup developed during planning is separate from this repository.

## License

A license has not yet been selected. Do not assume permission to reuse or redistribute the code until one is added.
