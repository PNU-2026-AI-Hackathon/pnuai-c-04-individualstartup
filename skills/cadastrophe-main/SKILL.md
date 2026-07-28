---
name: cadastrophe-main
description: Generate, render, iterate, and export CAD models through the Cadastrophe Tauri desktop app and review UI. Use when Codex needs to turn a natural language CAD request into OpenSCAD/CadQuery/FreeCAD source, create or update a Cadastrophe session, render a preview, collect user feedback, export STL/metadata artifacts, or hand a rendered artifact to a dedicated visual judge subagent.
---

# Cadastrophe Main

Use this skill to drive the Cadastrophe text-to-CAD loop from the agent side.
Cadastrophe owns the Tauri desktop backend, WebView UI, preview rendering,
revision state, user feedback, and artifact export. The skill owns modeling
strategy, source generation, repair, and deciding when a visual review is needed.

## Available Surface

The current repository no longer ships a TypeScript MCP server or HTTP/WebSocket
bridge. The app backend surface is Tauri IPC commands and `cad_bridge_event`
snapshots. If future agent-facing tools expose the desktop backend, map them to
these app operations:

- `list_runtime_capabilities`: inspect available runtimes and export formats.
- `create_cad_session`: create a user-visible session and get a UI URL.
- `update_model_source`: attach generated CAD source as a revision.
- `render_preview`: run the selected runtime and create preview mesh artifacts.
- `get_session_state`: inspect canonical session, active revision, diagnostics, artifacts, conversation, and runs.
- `get_current_cad_session`: find the session currently viewed by the user in the UI.
- prompt composer / conversation: collect user feedback in the UI.
- `export_artifact`: export supported artifacts such as `stl` or `metadata`.

Current MVP evidence from this repository:

- Runtime kind: `openscad-wasm`.
- Source language: `openscad`.
- Preview artifact kind: `preview-mesh` JSON read through a Tauri command.
- Export formats: `stl`, `metadata`.
- OpenSCAD fallback parser previews independent `cube`, `sphere`, `cylinder`, and simple `translate(...)` primitives. More complex OpenSCAD may still run through WASM, but fallback preview warns that operations such as `union`, `difference`, `intersection`, `rotate`, `scale`, `minkowski`, and `hull` are not evaluated by the MVP parser.

Do not call IRIT-specific pipeline tools; they are not part of Cadastrophe.

## Workflow

1. Inspect runtime support when an agent-facing surface provides that operation, unless the session already exists and its runtime is known.
2. Reuse the user's visible session when possible. If the app reports a current session, update that session. Only create a session when there is no current UI-viewed session.
3. Make a compact modeling plan in assistant reasoning or response. Do not persist a separate coarse plan unless the Cadastrophe protocol adds one.
4. Generate source for the selected runtime. For the current MVP, prefer simple OpenSCAD CSG and parameters at the top of the file.
5. Call `update_model_source` with the full source and explicit parameters when useful.
6. Call `render_preview`. Treat diagnostics as authoritative.
7. If diagnostics contain errors, repair the source and repeat from `update_model_source`.
8. When the preview renders, use the UI conversation/prompt flow if the user needs to approve, reject, edit parameters, or comment.
9. If the user rejects or edits parameters, update the source or parameter values and render again.
10. Export only after preview diagnostics are OK and the user has approved or the task explicitly asks for a raw export. Call `export_artifact` for `stl` or `metadata`.

## OpenSCAD Authoring

Follow these rules for the current MVP:

- Use Z-up coordinates. Interpret `[x, y, z]` as `[width, depth, height]`.
- Put user-tunable values at the top as assignments with optional parameter metadata:

```openscad
width = 40; // @param min=10 max=120 step=1 label=Width
depth = 28; // @param min=10 max=120 step=1 label=Depth
height = 12; // @param min=4 max=80 step=1 label=Height
```

- Prefer `cube`, `sphere`, `cylinder`, `translate`, and straightforward CSG.
- Keep object names and comments meaningful, but avoid nonstandard dependencies.
- Avoid file IO, `include`, `use`, or host-dependent paths.
- For booleans, avoid perfectly coplanar or coincident faces. Add small overlap where two solids join or one solid cuts another.
- If a preview warning says the MVP parser is showing independent primitives, explain that the UI preview may be approximate unless WASM produced STL successfully.

## Validation

Use three levels of validation:

1. Runtime diagnostics from `render_preview`. Fix all errors before progressing.
2. User-visible review through the desktop UI conversation/prompt flow.
3. Optional visual judge handoff only when a rendered image artifact or explicit image path is available.

Do not perform independent VLM judgment in the main modeling agent. If the
Cadastrophe protocol later returns a contract such as `cadastrophe.vlm_judge.v1`
with `rendered_images.grid` or another concrete image path, start a separate
subagent using the `cadastrophe-vlm-judge` skill and pass only the contract plus
the request to return strict JSON. Use the subagent result as review evidence,
then revise the model if it fails.

In the current repository, there is no persisted rendered PNG/grid artifact and no
app command for submitting a VLM judge result. Therefore, visual judge handoff is
a future-compatible conditional step, not a mandatory current gate.

## Feedback Handling

When user feedback arrives through the UI conversation:

- `approved`: export requested artifacts or finish with the approved session/revision.
- `rejected`: use `reason` and `comment` to revise source, then render again.
- `edited`: apply `parameterUpdates` or regenerate source, then render again.
- `commented`: decide whether the comment requires a source change; if so, revise and render again.

Always keep revision lineage by passing the previous active revision as
`parentRevisionId` when updating a model after feedback.
