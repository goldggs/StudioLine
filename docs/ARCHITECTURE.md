# Architecture notes

These notes record the current direction. They are not a claim that the behavior is implemented.

## Boundaries

1. The Rust app owns local project data, coordinates agent requests, and serves the browser interface.
2. The Luau plugin interacts with the open Studio session through a local WebSocket connection.
3. Separate Arena chats access the agent-facing API through a tunnel. They are independent chats, not simulated roles in a single chat.
4. The local user interface controls the project. Agent-facing access should not expose local administrative controls.

The initial product targets one active project and one connected Studio session at a time. Multiple agents can work on that session.

## Workspace

- Shared chat and direct messages.
- Self-described agent names and roles.
- An optional manager. The user approves manager requests, and each project has at most one manager.
- Tasks, references, approvals, activity, and a project pause control.
- Project commands and Studio/development commands are categories, not role restrictions.

## Approvals and interrupts

The default experience should allow routine work and prompt for actions classified as risky. The user can edit agent permissions or enable project-wide unrestricted mode. Exact classifications are not yet decided.

Urgent messages can hold an agent's next command until it acknowledges the message and decides whether to cancel, revise, or continue. This is cooperative at the next interaction, not a guarantee that an agent or an already-running command can be interrupted.

## Connection and privacy

- Tunnel addresses and agent credentials should be hidden by default and revealed or copied deliberately.
- Each agent should have its own revocable credential.
- When a temporary tunnel address changes, the user should be informed and be able to copy updated agent prompts.
- Logs and diagnostic reports should avoid credentials, connection prompts, tunnel addresses, and project content by default.
- Prevent the public agent interface from exposing local administrative endpoints. Local endpoints also need protection from unauthorized browser requests.

## Open design questions

- Exact Studio capabilities and limits, including visual inspection and playtesting.
- Agent delivery behavior while a chat is inactive or a tool call is running.
- Versioning scope, conflict detection, and what can be restored.
- Default approval rules and how to handle arbitrary Luau execution.
- Wally package resolution and conversion into Studio instances.
- Windows installer, plugin discovery, signed distribution, and tunnel lifecycle.
