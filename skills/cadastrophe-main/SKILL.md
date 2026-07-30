---
name: cadastrophe-main
description: Generate, render, iterate, and export CAD models through the Cadastrophe Milestone 3 CLI workflow and review UI. Use when Codex needs to turn a natural language CAD request into OpenSCAD source for openscad-wasm, commit a CadModelPlan, apply source, render previews, finalize artifacts, respond to structural/VLM failure reports, or hand a rendered artifact contract to the dedicated cadastrophe-vlm-judge subagent.
---

# Cadastrophe Main

Use this skill to drive the Cadastrophe text-to-CAD workflow from the agent side.
Cadastrophe owns canonical session state in the Tauri/Rust backend, SQLite
workflow tables, revision/artifact storage, preview rendering, finalization, and
run-event persistence. The main modeling agent owns planning, source generation,
runtime repair, and outer-loop refinement.

## Agent Surface

Milestone 3 exposes agent tools as executable CLIs, not an MCP server. All
commands use JSON output by default, support `--pretty` only for human
inspection, and support `--app-data-dir <dir>` for fixtures or tests.

Use these exact command names:

- `cadastrophe-session-current`: inspect current session id, active revision, selected runtime, and app data path.
- `cadastrophe-session-state --session <id>`: inspect session/revision/artifact/run/workflow state JSON.
- `cadastrophe-plan-commit --session <id> --run <id> --plan <file>`: validate and persist a `CadModelPlan`.
- `cadastrophe-source-apply --session <id> --run <id> --source <file> --language openscad`: append a source revision linked to the run.
- `cadastrophe-preview-render --session <id> --revision <id>`: render preview and return diagnostics/artifact ids.
- `cadastrophe-artifact-export --session <id> --revision <id> --format stl`: export an artifact when explicitly needed.
- `cadastrophe-evaluate-structural --session <id> --revision <id> --plan <file>`: run the deterministic structural anchor directly when needed.
- `cadastrophe-finalize --session <id> --run <id> --revision <id>`: lock final artifacts, run structural anchor, render mandatory VLM image evidence, and either return a failure report or a pending VLM contract.
- `cadastrophe-vlm-submit --session <id> --run <id> --artifact <id> --report <file>`: submit a VLM judge report and append outer-loop pass/fail history.

Important CLI data fields include `nextAction`, `next_action`, `diagnostics`,
`failureReport`, `failure_report`, `artifactPaths`, `contractType`,
`vlmContract`, `finalArtifact`, and `renderedImageArtifact`. Treat non-zero exit
status and JSON error envelopes as authoritative.

## Required Workflow

1. Reuse the current UI-visible session when possible. Use `cadastrophe-session-current` or the provided session id.
2. Inspect `cadastrophe-session-state --session <id>` before retrying, and carry the latest structural or VLM `failureReport`/`failure_report` into the next attempt.
3. Create a `CadModelPlan` JSON file and commit it with `cadastrophe-plan-commit --session <id> --run <run_id> --plan <file>`.
4. Do not apply source, render, evaluate structural state, finalize, or submit VLM until plan commit succeeds for the same run.
5. Generate source for the selected runtime. Current primary support is OpenSCAD through `openscad-wasm`.
6. Call `cadastrophe-source-apply --session <id> --run <run_id> --source <file> --language openscad`.
7. Call `cadastrophe-preview-render --session <id> --revision <revision_id>`.
8. If runtime diagnostics contain errors, explain the cause briefly, repair the source, then repeat source apply and preview render.
9. When preview diagnostics pass, call `cadastrophe-finalize --session <id> --run <run_id> --revision <revision_id>`.
10. If finalization returns a structural `failure_report` or `next_action` of `outer_loop_refine_source`, do not run VLM. Use that report for a new plan/source attempt.
11. If finalization returns a `cadastrophe.vlm_judge.v1` contract, verify it includes `renderedImages.available: true` and a usable rendered PNG path, then hand that exact contract to a separate subagent using the `cadastrophe-vlm-judge` skill. Ask the subagent to return only strict JSON.
12. Save the judge JSON to a file and call `cadastrophe-vlm-submit --session <id> --run <run_id> --artifact <artifact_id> --report <file>`.
13. If VLM submit records failure, refine from the VLM failure report and repeat from plan commit. If it passes, finish with the final revision/artifact ids.

RAG is optional for Milestone 3. Omitting RAG does not change the required
Plan -> source apply -> preview render -> finalize -> structural/VLM sequence.

## Plan Contract

The plan file must be runtime-neutral JSON shaped as `CadModelPlan`:

- `schemaVersion`: currently `cad_model_plan.v1`.
- `summary`: concise model intent.
- `mainComponent`: object with `name`, `purpose`, and optional `requiredFeatures`.
- `supportingComponents`: array of component objects.
- `expectedAspectRatio`: `{ "x": number, "y": number, "z": number, "tolerance": number }`.
- `sourceLanguage`: currently `openscad`.
- `runtimeConstraints`: runtime, required/forbidden features, and optional `mainComponentAnnotation`.

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
finalization returns a contract with:

```json
{
  "contractType": "cadastrophe.vlm_judge.v1",
  "artifactId": "final-stl-artifact-id",
  "passThreshold": 0.8,
  "artifact": {
    "format": "stl",
    "relativePath": "artifacts/session/revision/final.stl",
    "sha256": "..."
  },
  "renderedImages": {
    "available": true,
    "format": "png",
    "path": "/absolute/path/render-grid.png",
    "viewMode": "9-view",
    "views": ["Front-Left-Top", "Front", "Front-Right-Top", "Left", "Top", "Right", "Bottom", "Back", "Back-Right-Top"]
  }
}
```

Start a separate subagent with the `cadastrophe-vlm-judge` skill and pass the
contract plus the user's original request. The subagent must return a strict
`cadastrophe.vlm_judge_report.v1` JSON report. Submit that report through
`cadastrophe-vlm-submit`; do not treat the visual result as accepted until the
submit command records pass/fail in workflow state.

## Completion Discipline

A Cadastrophe run is not complete just because source was generated. Finish only
after the persisted workflow has advanced through plan commit, source apply,
preview render, finalization/structural anchor, and the required VLM submit when
there is pending VLM. If any gate fails, preserve the machine-readable failure
report and use it as the next outer-loop prompt context.
