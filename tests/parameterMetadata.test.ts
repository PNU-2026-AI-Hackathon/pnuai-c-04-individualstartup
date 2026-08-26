import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Diagnostics } from "../ui/src/components/RevisionPanels";
import type { CadParameter, CadSessionState } from "../ui/src/protocol";
import { createRenderFailureDiagnostics } from "../ui/src/runtime/openscadRuntime";
import { parameterHashInput } from "../ui/src/runtime/parameterMetadata";

test("parameter metadata hash input is stable regardless of source declaration order", () => {
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

test("invalid parameter metadata fails before rendering", () => {
  assert.throws(
    () => parameterHashInput([{ name: "width", type: "number", value: Number.NaN }]),
    /finite number/
  );
  assert.throws(
    () => parameterHashInput([{ name: "invalid-name", type: "number", value: 1 }]),
    /valid OpenSCAD identifier/
  );
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
