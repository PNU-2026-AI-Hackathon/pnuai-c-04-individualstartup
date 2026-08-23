import assert from "node:assert/strict";
import test from "node:test";
import type { CadBackendClient } from "../ui/src/backendClient";
import type { CadRevision, CadSessionState, CreateCadSessionResult } from "../ui/src/protocol";
import { duplicateSessionWithPreview } from "../ui/src/sessionWorkflow";

test("duplicating a session applies its copied source before rendering its preview", async () => {
  const duplicatedState = duplicatedSessionState();
  const calls: string[] = [];
  const backend: Pick<CadBackendClient, "duplicateSession" | "markSessionViewed"> = {
    async duplicateSession({ sessionId }) {
      calls.push(`duplicate:${sessionId}`);
      return duplicateResult(duplicatedState);
    },
    async markSessionViewed(sessionId) {
      calls.push(`viewed:${sessionId}`);
      return duplicatedState;
    }
  };

  const result = await duplicateSessionWithPreview({
    backend,
    sessionId: "session-original",
    applySessionSnapshot(state) {
      calls.push(`snapshot:${state.session.id}:${state.activeRevision?.source}`);
    },
    async renderRevision(sessionId, revision) {
      calls.push(`render:${sessionId}:${revision?.id}:${revision?.source}`);
    }
  });

  assert.equal(result.state.activeRevision?.source, "cube([12, 8, 4]);");
  assert.deepEqual(calls, [
    "duplicate:session-original",
    "snapshot:session-copy:cube([12, 8, 4]);",
    "viewed:session-copy",
    "render:session-copy:revision-copy:cube([12, 8, 4]);"
  ]);
});

function duplicatedSessionState(): CadSessionState {
  const now = "2026-08-23T00:00:00.000Z";
  const revision = {
    id: "revision-copy",
    sessionId: "session-copy",
    sourceHash: "copied-source-hash",
    sourceLanguage: "openscad" as const,
    source: "cube([12, 8, 4]);",
    parameters: [],
    createdAt: now,
    diagnostics: { ok: true, elapsedMs: 0, items: [] },
    artifacts: [],
    artifactCount: 0,
    userEvents: [],
    runLinks: []
  } satisfies CadRevision;
  return {
    session: {
      id: "session-copy",
      createdAt: now,
      updatedAt: now,
      connectedUiClients: 0,
      title: "Original copy",
      titleSource: "user",
      activeRevisionId: revision.id,
      selectedRuntime: "openscad-wasm",
      status: "idle",
      revisions: [revision]
    },
    activeRevision: revision,
    messages: [],
    conversation: [],
    agentThreads: [],
    agentRuns: [],
    agentRunEvents: [],
    validationEvaluations: [],
    validationBatches: [],
    validationChecks: [],
    workflow: { plans: [], outerIterations: [], pendingVlm: [] }
  };
}

function duplicateResult(state: CadSessionState): CreateCadSessionResult {
  return {
    sessionId: state.session.id,
    uiUrl: `/sessions/${state.session.id}`,
    state
  };
}
