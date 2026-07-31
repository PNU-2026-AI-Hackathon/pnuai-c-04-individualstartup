---
name: cadastrophe-main
description: Generate and iterate CAD model code through the Cadastrophe app-owned workflow. Use when Codex needs to turn a natural language CAD request into a CadModelPlanDraft plus OpenSCAD source for openscad-wasm, apply source, finalize app-owned evaluation, respond to structural/VLM failure reports, or hand a rendered artifact contract to the dedicated cadastrophe-vlm-judge subagent.
---

# Cadastrophe Main

Use this skill to drive the Cadastrophe text-to-CAD workflow from the agent side.
Cadastrophe owns canonical session state in the Tauri/Rust backend, SQLite
workflow tables, revision/artifact storage, preview rendering, export,
structural evaluation, VLM result recording, finalization, and run-event
persistence. The main modeling agent owns planning, source generation, runtime
repair, and outer-loop refinement.

## Agent Surface

Use these exact command names:

- `cadastrophe-session-current`: inspect current session id, active revision, selected runtime, and app data path.
- `cadastrophe-session-state`: inspect the current session/revision/artifact/run/workflow state JSON.
- `cadastrophe-plan-commit --plan <file>`: commit a `CadModelPlanDraft` for the active run.
- `cadastrophe-source-apply --source <file>`: append OpenSCAD source for the active run; the app renders preview/STL automatically and returns diagnostics.
- `cadastrophe-finalize`: lock final artifacts for the active revision/run, run structural anchor, render mandatory VLM image evidence, and either return a failure report or a pending VLM handoff.

Important CLI data fields include `nextAction`, `next_action`, `diagnostics`,
`failureReport`, `failure_report`, `artifactPaths`, `contractType`,
`vlmContract`, `finalArtifact`, and `renderedImageArtifact`. Treat non-zero exit
status and JSON error envelopes as authoritative.

## Required Workflow

1. Reuse the current UI-visible session. Use `cadastrophe-session-current` only when you need to inspect app routing state.
2. Inspect `cadastrophe-session-state` before retrying, and carry the latest structural or VLM `failureReport`/`failure_report` into the next attempt.
3. Create a `CadModelPlanDraft` JSON file and commit it with `cadastrophe-plan-commit --plan <file>`.
4. Do not apply source or finalize until plan commit succeeds for the same run.
5. Generate source for the selected runtime. Current primary support is OpenSCAD through `openscad-wasm`.
6. Call `cadastrophe-source-apply --source <file>`.
7. Source apply triggers app-owned preview render and returns diagnostics. If runtime diagnostics contain errors, explain the cause briefly, repair the source, then repeat source apply.
8. When preview diagnostics pass, call `cadastrophe-finalize`.
9. Finalize runs export, structural anchor, VLM evidence rendering, and pending VLM persistence. If finalization returns a structural `failure_report` or `next_action` of `outer_loop_refine_source`, use that report for a new plan/source attempt.
10. If finalization returns a `cadastrophe.vlm_judge.v1` handoff, verify it includes `renderedImages.available: true` and a usable rendered PNG path, then hand the handoff, image path, and original user request to a separate subagent using the `cadastrophe-vlm-judge` skill. Ask the subagent to return only strict JSON.
11. Return the strict VLM judge report as the assistant message. The app consumes the report automatically and records pass/fail workflow state.
12. If the app-recorded VLM result fails, refine from the VLM failure report and repeat from plan commit. If it passes, finish with the final revision/artifact ids.

Plan -> source apply -> app preview -> finalize -> app structural/VLM sequence.

## Plan Contract

The plan file authored by the agent must be runtime-neutral JSON shaped as `CadModelPlanDraft`:

- `summary`: concise model intent.
- `mainComponent`: object with `name`, `purpose`, and optional `requiredFeatures`.
- `supportingComponents`: array of component objects.
- `expectedAspectRatio`: `{ "x": number, "y": number, "z": number }`.

For OpenSCAD, include a source header matching the committed main component:

```openscad
// @main_component wall_bracket
```

## OpenSCAD Authoring

- Use Z-up coordinates. Interpret `[x, y, z]` as `[width, depth, height]`.
- Put tunable numeric values at the top with parameter comments when useful:

```openscad
width = 40; // @param min=10 max=120 step=1 label=Width
depth = 28; // @param min=10 max=120 step=1 label=Depth
height = 12; // @param min=4 max=80 step=1 label=Height
```

- Use OpenSCAD CSG normally through `openscad-wasm`, including `cube`, `sphere`, `cylinder`, `translate`, `rotate`, `union`, and `difference`.
- Avoid file IO, `include`, `use`, host-dependent paths, or nonstandard dependencies.
- Avoid coplanar boolean faces; add small overlaps where parts join or one solid cuts another.

## VLM Handoff

The main agent must not perform independent VLM judgment. Only hand off when
finalization returns a minimal handoff with:

```json
{
  "contractType": "cadastrophe.vlm_judge.v1",
  "handoff": "VLM Judge Handoff needed.",
  "renderedImages": {
    "available": true,
    "path": "/absolute/path/render-grid.png"
  }
}
```

Start a separate subagent with the `cadastrophe-vlm-judge` skill and pass the
handoff, rendered image path, and user's original request. The subagent must
return a strict `cadastrophe.vlm_judge_report.v1` JSON report. Return that report as the
assistant message so the app can consume it and record pass/fail in workflow
state.

## Completion Discipline

A Cadastrophe run is not complete just because source was generated. Finish only
after the persisted workflow has advanced through plan commit, source apply,
app preview render, finalization/structural anchor, and the required app-owned
VLM result recording when there is pending VLM. If any gate fails, preserve the
machine-readable failure report and use it as the next outer-loop prompt
context.
