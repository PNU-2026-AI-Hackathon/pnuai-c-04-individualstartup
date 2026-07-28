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
  "contract_type": "cadastrophe.vlm_judge.v1",
  "artifact_id": "artifact-or-revision-id",
  "request": "original natural language CAD request",
  "source_language": "openscad",
  "plan": {
    "summary": "brief modeling intent",
    "components": [
      {"name": "base", "description": "rectangular base"}
    ],
    "expected_aspect_ratio": [1, 1, 0.4]
  },
  "artifact_paths": {
    "stl": "/absolute/path/model.stl",
    "obj": "/absolute/path/model.obj",
    "preview_mesh": "/absolute/path/preview.json"
  },
  "rendered_images": {
    "available": true,
    "grid": "/absolute/path/render-grid.png",
    "views": ["front", "top", "right"]
  },
  "pass_threshold": 7
}
```

If the contract lacks usable rendered image evidence, return a failing report
explaining that the visual artifact is missing. Do not invent a render pipeline
unless the parent explicitly provides one.

## Judgment Procedure

1. Inspect the rendered image path in `rendered_images.grid` first.
2. Enumerate every visible component, using names from `plan.components` when possible.
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

Set `composite` to the sum of the three scores. Set `passed` to true only when
`composite >= pass_threshold` and no major requested feature is visibly missing.

## Output

Return exactly this JSON shape:

```json
{
  "artifact_id": "artifact-or-revision-id",
  "passed": true,
  "enumeration": [
    {
      "plan_name": "base",
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
  "diagnostic": "Short, concrete feedback describing any visible mismatch."
}
```

Rules:

- Output no prose outside the JSON object.
- `artifact_id` must exactly match the contract.
- `composite` must equal `structure + components + proportions`.
- `passed` must be consistent with `pass_threshold`.
- Mention every major requested component in `enumeration`, including absent components.
- Keep `diagnostic` actionable for the modeling agent.
