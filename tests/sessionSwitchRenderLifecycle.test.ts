import assert from "node:assert/strict";
import test from "node:test";
import {
  isOpenScadRenderCanceled,
  renderOpenScadInWorker,
  resetOpenScadRuntimeForSessionSwitch,
  type OpenscadRenderRequest,
  type OpenscadRenderResult
} from "../ui/src/runtime/openscadRuntime";
import type { OpenscadWorkerResponse } from "../ui/src/runtime/openscadWorker";

test("session switch cancels pending render and ignores stale worker completion", async () => {
  const originalWorker = globalThis.Worker;
  MockWorker.instances.length = 0;
  globalThis.Worker = MockWorker as unknown as typeof Worker;
  resetOpenScadRuntimeForSessionSwitch();

  try {
    const committedSessions: string[] = [];
    const sessionA = renderOpenScadInWorker(renderRequest("session-a", "revision-a"), () => undefined)
      .then((result) => {
        committedSessions.push(result.sessionId);
        return result;
      });
    const workerA = MockWorker.instances.at(-1);
    assert.ok(workerA);
    assert.equal(workerA.messages[0]?.type, "render");
    assert.equal(workerA.messages[0]?.sessionId, "session-a");
    assert.equal(workerA.messages[0]?.revisionId, "revision-a");
    assert.equal(workerA.messages[0]?.sourceHash, "source-session-a");
    assert.equal(workerA.messages[0]?.parameterHash, "params-session-a");

    resetOpenScadRuntimeForSessionSwitch();
    await assert.rejects(sessionA, (caught) => isOpenScadRenderCanceled(caught));
    assert.equal(workerA.terminated, true);

    const sessionB = renderOpenScadInWorker(renderRequest("session-b", "revision-b"), () => undefined)
      .then((result) => {
        committedSessions.push(result.sessionId);
        return result;
      });
    const workerB = MockWorker.instances.at(-1);
    assert.ok(workerB);
    assert.notEqual(workerA, workerB);

    workerA.emit(renderedResponse("session-a", "revision-a"));
    workerB.emit({
      type: "initialized",
      token: workerB.messages[0].token,
      sessionId: "session-b",
      revisionId: "revision-b"
    });
    workerB.emit(renderedResponse("session-b", "revision-b", workerB.messages[0].token));

    const resultB = await sessionB;
    assert.equal(resultB.sessionId, "session-b");
    assert.deepEqual(committedSessions, ["session-b"]);
  } finally {
    resetOpenScadRuntimeForSessionSwitch();
    globalThis.Worker = originalWorker;
  }
});

function renderRequest(sessionId: string, revisionId: string): OpenscadRenderRequest {
  return {
    sessionId,
    revisionId,
    source: `cube([1, 1, 1]); // ${sessionId}`,
    parameters: [],
    sourceHash: `source-${sessionId}`,
    parameterHash: `params-${sessionId}`
  };
}

function renderedResponse(
  sessionId: string,
  revisionId: string,
  token = 999
): OpenscadWorkerResponse {
  return {
    type: "rendered",
    token,
    ...renderedResult(sessionId, revisionId)
  };
}

function renderedResult(sessionId: string, revisionId: string): OpenscadRenderResult {
  return {
    sessionId,
    revisionId,
    diagnostics: { ok: true, elapsedMs: 12, items: [] },
    mesh: { vertices: [], normals: [], indices: [] },
    stlBytes: new Uint8Array([1, 2, 3]),
    sourceHash: `source-${sessionId}`,
    parameterHash: `params-${sessionId}`,
    stlSha256: `stl-${sessionId}`
  };
}

class MockWorker {
  static instances: MockWorker[] = [];

  readonly messages: any[] = [];
  readonly listeners = new Set<(event: MessageEvent<OpenscadWorkerResponse>) => void>();
  terminated = false;

  constructor() {
    MockWorker.instances.push(this);
  }

  addEventListener(type: string, listener: (event: MessageEvent<OpenscadWorkerResponse>) => void) {
    if (type === "message") this.listeners.add(listener);
  }

  removeEventListener(type: string, listener: (event: MessageEvent<OpenscadWorkerResponse>) => void) {
    if (type === "message") this.listeners.delete(listener);
  }

  postMessage(message: unknown) {
    this.messages.push(message);
  }

  terminate() {
    this.terminated = true;
    this.listeners.clear();
  }

  emit(data: OpenscadWorkerResponse) {
    for (const listener of this.listeners) {
      listener({ data } as MessageEvent<OpenscadWorkerResponse>);
    }
  }
}
