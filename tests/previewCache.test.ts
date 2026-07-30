import assert from "node:assert/strict";
import test from "node:test";
import {
  isLatestRenderGeneration,
  matchesOpenscadPreviewCache,
  nextRenderGeneration,
  type OpenscadPreviewCacheKey,
  type OpenscadRenderResult
} from "../ui/src/runtime/openscadRuntime";

const key: OpenscadPreviewCacheKey = {
  sessionId: "session-a",
  revisionId: "revision-a",
  sourceHash: "source-a",
  parameterHash: "params-a"
};

const cache: OpenscadRenderResult = {
  ...key,
  diagnostics: { ok: true, elapsedMs: 12, items: [] },
  mesh: { vertices: [0, 0, 0], normals: [0, 0, 1], indices: [0] },
  stlBytes: new Uint8Array([1, 2, 3]),
  stlSha256: "stl-a"
};

test("preview cache keys include session, revision, source hash, and parameter hash", () => {
  assert.equal(matchesOpenscadPreviewCache(cache, key), true);
  assert.equal(matchesOpenscadPreviewCache(cache, { ...key, sessionId: "session-b" }), false);
  assert.equal(matchesOpenscadPreviewCache(cache, { ...key, revisionId: "revision-b" }), false);
  assert.equal(matchesOpenscadPreviewCache(cache, { ...key, sourceHash: "source-b" }), false);
  assert.equal(matchesOpenscadPreviewCache(cache, { ...key, parameterHash: "params-b" }), false);
});

test("render generation helpers reject stale draft renders", () => {
  const first = nextRenderGeneration(0);
  const second = nextRenderGeneration(first);
  assert.equal(isLatestRenderGeneration(second, second), true);
  assert.equal(isLatestRenderGeneration(second, first), false);
  assert.equal(isLatestRenderGeneration(second, undefined), true);
});
