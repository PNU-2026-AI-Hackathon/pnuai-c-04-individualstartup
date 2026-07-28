import assert from "node:assert/strict";
import test from "node:test";
import type { CadBackendClient } from "../ui/src/backendClient";
import type {
  CadBridgeEvent,
  CadSessionState,
  CreateAgentRunResult,
  CreateCadSessionResult,
  CurrentCadSessionResult
} from "../ui/src/protocol";

test("Tauri backend client contract exposes semantic session and event shapes", async () => {
  const tauri = new MockCadBackendClient();

  await assertClientShape(tauri);
});

async function assertClientShape(client: CadBackendClient) {
  const created = await client.createSession({ title: "Transport contract" });
  assertSessionResultShape(created);

  const current = await client.getCurrentSession();
  assertCurrentSessionShape(current);

  const observedSnapshots: CadSessionState[] = [];
  const unsubscribe = client.subscribeSession(created.sessionId, {
    onStatus: () => undefined,
    onSnapshot: (state) => observedSnapshots.push(state),
    onError: (error) => {
      throw error;
    }
  });

  const started = await client.createAgentRun({
    sessionId: created.sessionId,
    prompt: "Create a bracket.",
    revisionId: created.state.session.activeRevisionId
  });
  assertAgentRunShape(started);
  assert.equal(observedSnapshots.at(-1)?.session.id, created.sessionId);
  assert.equal(observedSnapshots.at(-1)?.agentRuns.at(-1)?.id, started.run.id);
  unsubscribe();
}

function assertSessionResultShape(result: CreateCadSessionResult) {
  assert.equal(typeof result.sessionId, "string");
  assert.equal(typeof result.uiUrl, "string");
  assert.equal(result.state.session.id, result.sessionId);
  assert.equal(result.state.session.selectedRuntime, "openscad-wasm");
  assert.equal(result.state.activeRevision?.sourceLanguage, "openscad");
}

function assertCurrentSessionShape(result: CurrentCadSessionResult) {
  assert.equal(typeof result.sessionId, "string");
  assert.equal(result.state?.session.id, result.sessionId);
}

function assertAgentRunShape(result: CreateAgentRunResult) {
  assert.equal(result.message.role, "user");
  assert.equal(result.run.status, "queued");
  assert.equal(result.state.agentRuns.at(-1)?.id, result.run.id);
  assert.equal(result.state.conversation.at(-1)?.id, result.message.id);
}

class MockCadBackendClient implements CadBackendClient {
  private state: CadSessionState | undefined;
  private listeners = new Set<(event: CadBridgeEvent) => void>();

  async createSession(input: { title?: string }): Promise<CreateCadSessionResult> {
    this.state = sampleState(input.title ?? "Untitled CAD session");
    const result = {
      sessionId: this.state.session.id,
      uiUrl: `/sessions/${this.state.session.id}`,
      state: this.state
    };
    this.emit("session.created");
    return result;
  }

  async getCurrentSession(): Promise<CurrentCadSessionResult> {
    return {
      sessionId: this.requireState().session.id,
      uiUrl: `/sessions/${this.requireState().session.id}`,
      state: this.requireState()
    };
  }

  async getSessionState(): Promise<CadSessionState> {
    return this.requireState();
  }

  async markSessionViewed(): Promise<CadSessionState> {
    return this.requireState();
  }

  async updateModelSource(): Promise<{ revisionId: string; state: CadSessionState }> {
    return {
      revisionId: this.requireState().session.activeRevisionId ?? "revision-1",
      state: this.requireState()
    };
  }

  async renderPreview(): Promise<{ state: CadSessionState }> {
    return { state: this.requireState() };
  }

  async updateParameters(): Promise<CadSessionState> {
    return this.requireState();
  }

  async createAgentRun(input: { sessionId: string; prompt: string; revisionId?: string }): Promise<CreateAgentRunResult> {
    const state = this.requireState();
    const now = new Date().toISOString();
    const message = {
      id: "tauri-message-1",
      sessionId: input.sessionId,
      revisionId: input.revisionId,
      role: "user" as const,
      content: input.prompt,
      createdAt: now,
      metadata: { source: "web-ui" }
    };
    const run = {
      id: "tauri-run-1",
      sessionId: input.sessionId,
      status: "queued" as const,
      prompt: input.prompt,
      createdAt: now,
      updatedAt: now
    };
    this.state = {
      ...state,
      conversation: [...state.conversation, message],
      agentRuns: [...state.agentRuns, run]
    };
    this.emit("agent.run.created");
    return { message, run, state: this.state };
  }

  async cancelAgentRun(): Promise<{ run: never; state: CadSessionState }> {
    throw new Error("No cancellable run in mock.");
  }

  async exportArtifact(): Promise<{ state: CadSessionState }> {
    return { state: this.requireState() };
  }

  async readPreviewMesh() {
    return { vertices: [], normals: [], indices: [] };
  }

  subscribeSession(
    sessionId: string,
    handlers: {
      onStatus: (status: "connecting" | "connected" | "disconnected") => void;
      onSnapshot: (state: CadSessionState) => void;
      onError: (error: unknown) => void;
    }
  ): () => void {
    handlers.onStatus("connected");
    const listener = (event: CadBridgeEvent) => {
      if (event.sessionId === sessionId) handlers.onSnapshot(event.state);
    };
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
      handlers.onStatus("disconnected");
    };
  }

  private emit(type: CadBridgeEvent["type"]) {
    const state = this.requireState();
    const event: CadBridgeEvent = {
      id: `tauri-event-${Date.now()}`,
      type,
      sessionId: state.session.id,
      createdAt: new Date().toISOString(),
      state
    };
    for (const listener of this.listeners) listener(event);
  }

  private requireState(): CadSessionState {
    if (!this.state) throw new Error("Mock session has not been created.");
    return this.state;
  }
}

function sampleState(title: string): CadSessionState {
  const now = new Date().toISOString();
  return {
    session: {
      id: "session-1",
      createdAt: now,
      updatedAt: now,
      connectedUiClients: 0,
      title,
      activeRevisionId: "revision-1",
      selectedRuntime: "openscad-wasm",
      status: "idle",
      revisions: [
        {
          id: "revision-1",
          sourceLanguage: "openscad",
          createdAt: now,
          diagnostics: { ok: true, elapsedMs: 0, items: [] },
          artifactCount: 0
        }
      ]
    },
    activeRevision: {
      id: "revision-1",
      sessionId: "session-1",
      sourceLanguage: "openscad",
      source: "cube([1, 1, 1]);",
      parameters: [],
      createdAt: now,
      diagnostics: { ok: true, elapsedMs: 0, items: [] },
      artifactCount: 0,
      artifacts: [],
      userEvents: []
    },
    messages: [],
    conversation: [],
    agentRuns: []
  };
}
