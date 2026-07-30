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
  Worker and exports the same STL bytes used for preview; CLI/agent commands
  invoke the same WASM package through Node.
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

The desktop backend uses Codex. Packaged macOS app launches do not always inherit
your interactive PATH, so the backend combines the current PATH, the app-adjacent
CLI directory, `CADASTROPHE_CODEX_EXTRA_PATHS`, and the login shell PATH before
starting the Codex child process.
