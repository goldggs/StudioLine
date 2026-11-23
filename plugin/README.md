# Studio plugin

The app creates a local `.rbxmx` plugin from `src/StudioLine.server.luau` and inserts a per-installation key when the user chooses to install it. The plugin key must not be committed, placed in logs, or shared. The unfilled source is not a runnable installation.

The plugin connects to the local app with a WebSocket after the user opens a project and clicks Connect. This first version supports a read-only place inspection command. Actual Studio behavior must be tested in Roblox Studio.
