---
name: cadastrophe-vlm-judge
description: Visually judge a Cadastrophe CAD artifact from a minimal VLM handoff in a dedicated subagent, using rendered image evidence and returning only the strict JSON report. Use only when the parent agent provides a Cadastrophe VLM handoff, rendered image path, and the natural language request.
---

# Cadastrophe VLM Judge

Use this skill only inside a dedicated visual judge subagent. Judge the rendered
Cadastrophe artifact against the user's natural language CAD request. Do not
modify files, call modeling tools, update sessions, or repair source.

Return only the required JSON object.

## Required Handoff

Expect a minimal handoff shaped like this:

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

The parent must also provide the user's original CAD request. If the handoff
lacks usable rendered image evidence, return a failing report
explaining that the visual artifact is missing. Do not invent a render pipeline
unless the parent explicitly provides one.

## Judgment Procedure

1. Inspect the rendered image path in `renderedImages.path` first.
2. Enumerate every visible component using names from the user's request when possible.
3. Compare visible components against the request. Look for missing parts, wrong counts, wrong placement, implausible proportions, and visible contradictions.
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
`score >= 0.8` and no major requested feature is visibly missing. The app may
apply its own persisted threshold again when it consumes the report.

## Output

Return exactly this JSON shape:

```json
{
  "contractType": "cadastrophe.vlm_judge_report.v1",
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
- If the handoff includes `runId` or `artifactId`, echo them exactly. If it does not, omit them; the app owns pending VLM state and will attach the IDs.
- `score` must be a number from `0.0` to `1.0`.
- `composite` must equal `structure + components + proportions`.
- `score` must equal `composite / 9.0`, rounded only as needed.
- `passed` must be consistent with the default `0.8` threshold.
- Mention every major requested component in `enumeration`, including absent components.
- Keep `diagnostic` actionable for the modeling agent.
- If `passed` is false, include a non-null `failureReport` with `contractType: "cadastrophe.failure_report.v1"`, a concrete `reason`, and `nextAction: "outer_loop_refine_source"`.
