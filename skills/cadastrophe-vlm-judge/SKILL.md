---
name: cadastrophe-vlm-judge
description: Visually judge a Cadastrophe CAD artifact from a VLM judge contract in a dedicated subagent, using rendered image evidence and returning only the strict JSON report. Use only when the parent agent provides a Cadastrophe visual judge contract with the natural language request, CAD plan or source summary, artifact paths, rendered image paths, and pass threshold.
---

# Cadastrophe VLM Judge

Use this skill only inside a dedicated visual judge subagent. Judge the rendered
Cadastrophe artifact against the user's natural language CAD request. Do not
modify files, call modeling tools, update sessions, or repair source.

Return only the required JSON object.

## Required Contract

Expect a contract shaped like this:

```json
{
  "contractType": "cadastrophe.vlm_judge.v1",
  "sessionId": "session-id",
  "runId": "run-id",
  "revisionId": "revision-id",
  "artifactId": "final-stl-artifact-id",
  "passThreshold": 0.8,
  "prompt": "Judge whether the final CAD artifact visually satisfies the committed CadModelPlan.",
  "plan": {
    "schemaVersion": "cad_model_plan.v1",
    "summary": "brief modeling intent",
    "mainComponent": {
      "name": "base",
      "purpose": "rectangular base"
    },
    "supportingComponents": [],
    "expectedAspectRatio": { "x": 1, "y": 1, "z": 0.4, "tolerance": 0.2 },
    "sourceLanguage": "openscad",
    "runtimeConstraints": { "runtime": "openscad-wasm" }
  },
  "artifact": {
    "format": "stl",
    "relativePath": "artifacts/session/revision/model.stl",
    "sha256": "...",
    "bytes": 12345
  },
  "renderedImages": {
    "available": true,
    "artifactId": "render-grid-artifact-id",
    "format": "png",
    "relativePath": "artifacts/session/revision/render-grid.png",
    "path": "/absolute/path/render-grid.png",
    "sha256": "...",
    "bytes": 23456,
    "viewMode": "9-view",
    "views": ["Front-Left-Top", "Front", "Front-Right-Top", "Left", "Top", "Right", "Bottom", "Back", "Back-Right-Top"]
  }
}
```

If the contract lacks usable rendered image evidence, return a failing report
explaining that the visual artifact is missing. Do not invent a render pipeline
unless the parent explicitly provides one.

## Judgment Procedure

1. Inspect the rendered image path in `renderedImages.path` first.
2. Enumerate every visible component, using names from `plan.mainComponent` and `plan.supportingComponents` when possible.
3. Compare visible components against the request and plan. Look for missing parts, wrong counts, wrong placement, implausible proportions, and visible contradictions.
4. Check inter-view consistency when multiple labeled views are present.
5. Score structure, components, and proportions using the rubric below.
6. Return only JSON.

## Scoring Rubric

Structure:

- 0: No meaningful resemblance to the request.
- 1: Similar broad category but wrong type.
- 2: Correct type with visible discrepancies.
- 3: Overall shape type matches the request.

Components:

- 0: Major requested components are missing.
- 1: Some requested components are present.
- 2: Most requested components are present.
- 3: All requested components are present.

Proportions:

- 0: Relative sizes or arrangement are clearly wrong.
- 1: Approximately reasonable with visible inaccuracies.
- 2: Plausible with minor issues.
- 3: Natural and consistent with the request.

Set `score` to `composite / 9.0`. Set `passed` to true only when
`score >= passThreshold` and no major requested feature is visibly missing.

## Output

Return exactly this JSON shape:

```json
{
  "contractType": "cadastrophe.vlm_judge_report.v1",
  "runId": "run-id",
  "artifactId": "final-stl-artifact-id",
  "score": 0.89,
  "passed": true,
  "findings": [
    {
      "severity": "info",
      "message": "The main rectangular base is visible across the labeled views."
    }
  ],
  "enumeration": [
    {
      "planName": "base",
      "observed": "One rectangular base is visible in the front, top, and right views."
    }
  ],
  "inconsistencies": [],
  "scores": {
    "structure": 3,
    "components": 3,
    "proportions": 2
  },
  "composite": 8,
  "diagnostic": "Short, concrete feedback describing any visible mismatch.",
  "failureReport": null
}
```

Rules:

- Output no prose outside the JSON object.
- `contractType` must be exactly `cadastrophe.vlm_judge_report.v1`.
- `runId` and `artifactId` must exactly match the contract.
- `score` must be a number from `0.0` to `1.0`.
- `composite` must equal `structure + components + proportions`.
- `score` must equal `composite / 9.0`, rounded only as needed.
- `passed` must be consistent with `passThreshold`.
- Mention every major requested component in `enumeration`, including absent components.
- Keep `diagnostic` actionable for the modeling agent.
- If `passed` is false, include a non-null `failureReport` with `contractType: "cadastrophe.failure_report.v1"`, a concrete `reason`, and `nextAction: "outer_loop_refine_source"`.
