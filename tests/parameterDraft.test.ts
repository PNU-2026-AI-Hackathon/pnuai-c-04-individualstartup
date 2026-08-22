import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Diagnostics } from "../ui/src/components/RevisionPanels";
import type { CadParameter, CadSessionState } from "../ui/src/protocol";
import {
  createRenderFailureDiagnostics,
  isLatestRenderGeneration,
  nextRenderGeneration
} from "../ui/src/runtime/openscadRuntime";
import {
  applyParameterValuesToSource,
  parameterHashInput,
  ParameterDraftError,
  updateParameterDraft
} from "../ui/src/runtime/parameterDraft";

test("updateParameterDraft changes only the requested parameter", () => {
  const parameters: CadParameter[] = [
    { name: "width", type: "number", value: 12 },
    { name: "label", type: "string", value: "front" }
  ];

  const updated = updateParameterDraft(parameters, "width", 18);

  assert.deepEqual(updated, [
    { name: "width", type: "number", value: 18 },
    { name: "label", type: "string", value: "front" }
  ]);
  assert.equal(parameters[0].value, 12);
});

test("updateParameterDraft keeps numeric OpenSCAD parameters finite and bounded", () => {
  const parameters: CadParameter[] = [
    { name: "width", type: "number", value: 12, min: 1, max: 20 },
    { name: "height", type: "number", value: 8 }
  ];

  assert.deepEqual(updateParameterDraft(parameters, "width", Number.NaN)[0].value, 12);
  assert.deepEqual(updateParameterDraft(parameters, "width", 24)[0].value, 20);
  assert.deepEqual(updateParameterDraft(parameters, "width", -4)[0].value, 1);
  assert.deepEqual(updateParameterDraft(parameters, "height", Number.POSITIVE_INFINITY)[1].value, 8);
});

test("applyParameterValuesToSource rewrites only matching @param declarations", () => {
  const source = [
    "width = 12; // @param min=1 max=40",
    "label = \"front\"; // @param",
    "height = 5;",
    "cube([width, height, 2]);"
  ].join("\n");
  const parameters: CadParameter[] = [
    { name: "width", type: "number", value: 18 },
    { name: "label", type: "string", value: "side" },
    { name: "missing", type: "boolean", value: true }
  ];

  assert.equal(
    applyParameterValuesToSource(source, parameters),
    [
      "width = 18; // @param min=1 max=40",
      "label = \"side\"; // @param",
      "height = 5;",
      "cube([width, height, 2]);"
    ].join("\n")
  );
});

test("parameter draft preview does not mutate revision count", () => {
  const revisions = [{ id: "revision-1" }, { id: "revision-2" }];
  const source = "width = 12; // @param min=1 max=40\ncube([width, 4, 2]);";
  const parameters: CadParameter[] = [{ name: "width", type: "number", value: 12 }];

  const nextParameters = updateParameterDraft(parameters, "width", 18);
  const nextSource = applyParameterValuesToSource(source, nextParameters);

  assert.equal(revisions.length, 2);
  assert.notEqual(nextParameters, parameters);
  assert.match(nextSource, /^width = 18;/);
});

test("parameter draft source rewrite rejects invalid OpenSCAD assignments", () => {
  assert.throws(
    () => applyParameterValuesToSource("bad = 0; // @param", [{ name: "bad", type: "number", value: Number.NaN }]),
    ParameterDraftError
  );
  assert.throws(
    () => updateParameterDraft([{ name: "width", type: "number", value: 12 }], "missing", 4),
    /not defined/
  );
});

test("parameter hash input is stable for draft render identity", () => {
  const left: CadParameter[] = [
    { name: "width", type: "number", value: 18 },
    { name: "label", type: "string", value: "side" }
  ];
  const right: CadParameter[] = [
    { name: "label", type: "string", value: "side" },
    { name: "width", type: "number", value: 18 }
  ];

  assert.equal(parameterHashInput(left), parameterHashInput(right));
});

test("latest draft render generation is the only committable draft", () => {
  const first = nextRenderGeneration(0);
  const latest = nextRenderGeneration(first);

  assert.equal(isLatestRenderGeneration(latest, latest), true);
  assert.equal(isLatestRenderGeneration(latest, first), false);
});

test("render failure diagnostics display origin, code, session, revision, and hashes", () => {
  const diagnostics = createRenderFailureDiagnostics({
    origin: "openscad-stderr",
    code: 1217160,
    message: "error: 1217160",
    sessionId: "session-1",
    revisionId: "revision-1",
    sourceHash: "source-hash",
    parameterHash: "parameter-hash"
  });
  const state = {
    activeRevision: { diagnostics }
  } as CadSessionState;

  const html = renderToStaticMarkup(React.createElement(Diagnostics, { state }));

  assert.match(html, /origin=openscad-stderr/);
  assert.match(html, /code=1217160/);
  assert.match(html, /session=session-1/);
  assert.match(html, /revision=revision-1/);
  assert.match(html, /sourceHash=source-hash/);
  assert.match(html, /parameterHash=parameter-hash/);
});
