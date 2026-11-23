# Testing

Automated tests currently cover local project storage, validation, concurrent requests, host and origin checks, shutdown authorization, the interface API, and a browser flow for project creation and shutdown. These are an early foundation, not a complete security review or red-team suite.

`./scripts/test.sh` runs Rust format, Clippy, Rust tests, the TypeScript build, and frontend unit tests in a Unix-like developer environment.

The browser tests deliberately create a project and shut down the running application. Run them only against an isolated test instance with its own `STUDIOLINE_DATA_DIR`. Install Playwright's Chromium in a temporary browser directory and run `npm run test:e2e` from `ui/`. Windows CI provisions an isolated instance, runs these tests, checks restart persistence and single-instance behavior, and tests the app executable. The workflow has not been run.

The earlier plugin connection was tested by the owner in Studio on Windows. New shutdown, export, and deletion behavior, agent access, integrated uninstall, and a comprehensive red-team suite remain to be validated or implemented.

An opt-in `tests/real-tunnel.cjs` test exercises a real Cloudflare tunnel and a simulated Studio client. It contacts Cloudflare and publishes a temporary address, so use only with a fresh isolated instance and data directory. For Linux development, set `STUDIOLINE_TEST_CLOUDFLARED` on the debug app to the SHA-256-verified Linux 2026.11.1 binary, then set `STUDIOLINE_NETWORK_TEST=1` and the same `STUDIOLINE_DATA_DIR` when running the test. It requires the development Node dependencies and Python `requests`. Do not run it against personal project data. The test stops the tunnel on success and attempts cleanup on failure.
