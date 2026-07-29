# cadastrophe
Text-to-CAD Tauri Desktop Workspace

## Current MVP

Tauri desktop CAD workspace:

- React/Vite web UI for agent prompts, conversation/run state, preview, source
  editing, revision timeline, parameters, diagnostics, and export.
- Tauri/Rust backend for sessions, model source revisions, user messages,
  runtime artifact persistence, artifact export, and agent run state.
- Tauri IPC commands and `cad_bridge_event` snapshots as the only app backend
  transport.
- Real OpenSCAD evaluation through `openscad-wasm`: the UI renders in a Web
  Worker and exports the same STL bytes used for preview; CLI/agent fallback
  commands invoke the same WASM package through Node.
- Real Codex process adapter as the default desktop agent path.

## Development

Install dependencies:

```sh
npm install
```

Run the desktop app:

```sh
npm run dev:tauri
```

Run the web UI only for frontend shell iteration:

```sh
npm run dev:ui
```

Backend-backed workflows require the Tauri runtime.

Build and type-check:

```sh
npm run build
```

Run verification:

```sh
npm run check
npm test
npm run test:rust
npm run build
npm run build:tauri
```

Run the real Codex adapter smoke, which requires a locally installed and
authenticated `codex` CLI:

```sh
npm run smoke:codex -- "Create a simple OpenSCAD sphere with radius 6."
```

The desktop backend uses Codex by default. Set
`CADASTROPHE_AGENT_ADAPTER=fake` only when you need deterministic fake-adapter
regression behavior. Packaged macOS app launches do not inherit your interactive
shell PATH, so the desktop backend also checks common Homebrew/npm locations such as
`/opt/homebrew/bin` and passes the expanded PATH to the Codex child process.
