import assert from "node:assert/strict";
import test from "node:test";
import type { CadBackendClient } from "../ui/src/backendClient";
import type {
  CadAgentStreamEvent,
  CadArtifact,
  CadBridgeEvent,
  CadSessionListItem,
  CadSessionState,
  CreateAgentRunResult,
  CreateCadSessionResult,
  CurrentCadSessionResult
} from "../ui/src/protocol";

test("Tauri backend client contract exposes semantic session and event shapes", async () => {
  const tauri = new ContractBackendClient();

  await assertClientShape(tauri);
});

async function assertClientShape(client: CadBackendClient) {
  const created = await client.createSession({ title: "Transport contract" });
  assertSessionResultShape(created);

  const current = await client.getCurrentSession();
  assertCurrentSessionShape(current);
  const listed = await client.listSessions({ includeArchived: true, query: "cube" });
  assertSessionListShape(listed.sessions[0]);
  assert.deepEqual(listed.searchFields, ["title", "source", "conversation"]);
  const renamed = await client.renameSession({ sessionId: created.sessionId, title: "Renamed contract" });
  assert.equal(renamed.session.title, "Renamed contract");
  const verified = await client.verifyArtifactFiles({ sessionId: created.sessionId });
  assert.equal(verified.checkedCount, 1);
  assert.deepEqual(verified.hashMismatchArtifactIds, []);
  assert.deepEqual(verified.orphanPaths, []);
  assert.deepEqual(verified.diagnostics, []);
  const openedArtifact = await client.openArtifact("artifact-1");
  assert.equal(openedArtifact.artifact.id, "artifact-1");
  assert.equal(openedArtifact.artifact.kind, "stl");
  assert.equal(typeof openedArtifact.path, "string");

  const observedSnapshots: CadSessionState[] = [];
  const unsubscribe = client.subscribeSession(created.sessionId, {
    onStatus: () => undefined,
    onSnapshot: (state) => observedSnapshots.push(state),
    onStream: () => undefined,
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
  const newConversation = await client.startNewAgentConversation(created.sessionId);
  assert.equal(newConversation.activeThread.externalThreadId, "contract-thread-1");
  const agentDiagnostics = await client.getAgentSessionDiagnostics(created.sessionId);
  assert.equal(agentDiagnostics.threads[0]?.thread.id, newConversation.activeThread.id);
  const cleanup = await client.cleanupAgentTransportEvents({
    sessionId: created.sessionId,
    maxEventsPerSession: 100
  });
  assert.equal(cleanup.deletedCount, 0);
  const active = await client.setActiveRevision({
    sessionId: created.sessionId,
    revisionId: created.state.session.activeRevisionId ?? "revision-1"
  });
  assert.equal(active.session.activeRevisionId, created.state.session.activeRevisionId);
  const restored = await client.restoreRevision({
    sessionId: created.sessionId,
    revisionId: created.state.session.activeRevisionId ?? "revision-1"
  });
  assert.equal(restored.state.activeRevision?.restoredFromRevisionId, created.state.session.activeRevisionId);
  const deletedArtifact = await client.deleteArtifact({
    sessionId: created.sessionId,
    artifactId: "artifact-1"
  });
  assert.equal(deletedArtifact.artifactId, "artifact-1");
  assert.equal(deletedArtifact.state.activeRevision?.artifacts.length, 0);
  const duplicated = await client.duplicateSession({ sessionId: created.sessionId });
  assert.equal(duplicated.sessionId, `${created.sessionId}-copy`);
  const deleted = await client.deleteSession(duplicated.sessionId);
  assert.equal(deleted.sessionId, duplicated.sessionId);
  assert.equal(typeof deleted.currentSessionId, "string");
  assert.equal(observedSnapshots.at(-1)?.session.id, created.sessionId);
  unsubscribe();
}

function assertSessionResultShape(result: CreateCadSessionResult) {
  assert.equal(typeof result.sessionId, "string");
  assert.equal(typeof result.uiUrl, "string");
  assert.equal(result.state.session.id, result.sessionId);
  assert.equal(result.state.session.selectedRuntime, "openscad-wasm");
  assert.equal(result.state.activeRevision?.sourceLanguage, "openscad");
  assert.equal(typeof result.state.session.revisions[0]?.sourceHash, "string");
  assert.ok(Array.isArray(result.state.session.revisions[0]?.runLinks));
  assert.equal(result.state.workflow.plans[0]?.plan.mainComponent.name, "contract_bracket");
  assert.equal(result.state.workflow.pendingVlm[0]?.contract.contractType, "cadastrophe.vlm_judge.v1");
  assert.equal(result.state.validationEvaluations[0]?.status, "queued");
  assert.equal(result.state.validationEvaluations[0]?.inputContract.contractType, "cadastrophe.vlm_evaluation_input.v1");
  assert.equal(result.state.validationBatches[0]?.status, "queued");
  assert.equal(result.state.validationChecks.length, 3);
  assert.deepEqual(result.state.validationChecks.map((check) => check.kind), ["structural", "dfm", "vlm"]);
}

function assertCurrentSessionShape(result: CurrentCadSessionResult) {
  assert.equal(typeof result.sessionId, "string");
  assert.equal(result.state?.session.id, result.sessionId);
}

function assertAgentRunShape(result: CreateAgentRunResult) {
  assert.equal(result.message.role, "user");
  assert.equal(result.run.status, "queued");
  assert.equal(result.run.externalAgent, "contract-test");
  assert.equal(result.state.agentRuns.at(-1)?.id, result.run.id);
  assert.equal(result.state.agentRunEvents.at(-1)?.runId, result.run.id);
  assert.equal(result.state.conversation.at(-1)?.id, result.message.id);
}

class ContractBackendClient implements CadBackendClient {
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

  async bootSession() {
    if (!this.state) {
      return this.createSession({ title: "Example OpenSCAD session" }).then((result) => ({
        ...result,
        isFirstRun: true,
        createdSession: true,
        shouldUseExampleSession: true,
        shouldAutoRender: true
      }));
    }
    return {
      sessionId: this.state.session.id,
      uiUrl: `/sessions/${this.state.session.id}`,
      state: this.state,
      isFirstRun: false,
      createdSession: false,
      shouldUseExampleSession: false,
      shouldAutoRender: false
    };
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

  async listSessions(input: { includeArchived?: boolean; query?: string } = {}) {
    const state = this.requireState();
    const item = sessionListItem(state);
    const query = input.query?.trim().toLowerCase();
    const matchesQuery = !query
      || item.title?.toLowerCase().includes(query)
      || state.activeRevision?.source.toLowerCase().includes(query)
      || state.conversation.some((message) => message.content.toLowerCase().includes(query));
    return {
      sessions: matchesQuery && (input.includeArchived || !item.archived) ? [item] : [],
      searchFields: ["title", "source", "conversation"]
    };
  }

  async renameSession(input: { sessionId: string; title: string }): Promise<CadSessionState> {
    const state = this.requireState();
    this.state = {
      ...state,
      session: { ...state.session, title: input.title, titleSource: "user" }
    };
    this.emit("session.updated");
    return this.state;
  }

  async archiveSession(): Promise<CadSessionState> {
    const state = this.requireState();
    this.state = {
      ...state,
      session: { ...state.session, archivedAt: new Date().toISOString() }
    };
    this.emit("session.updated");
    return this.state;
  }

  async deleteSession() {
    return { sessionId: this.requireState().session.id, currentSessionId: "session-1" };
  }

  async duplicateSession(input: { sessionId: string; title?: string }): Promise<CreateCadSessionResult> {
    this.state = {
      ...this.requireState(),
      session: {
        ...this.requireState().session,
        id: `${input.sessionId}-copy`,
        title: input.title ?? "Copy",
        titleSource: input.title ? "user" : this.requireState().session.titleSource
      }
    };
    return {
      sessionId: this.state.session.id,
      uiUrl: `/sessions/${this.state.session.id}`,
      state: this.state
    };
  }

  async updateModelSource(): Promise<{ revisionId: string; state: CadSessionState }> {
    return {
      revisionId: this.requireState().session.activeRevisionId ?? "revision-1",
      state: this.requireState()
    };
  }

  async setActiveRevision(input: { sessionId: string; revisionId: string }): Promise<CadSessionState> {
    const state = this.requireState();
    this.state = {
      ...state,
      session: { ...state.session, activeRevisionId: input.revisionId },
      activeRevision: state.activeRevision?.id === input.revisionId ? state.activeRevision : undefined
    };
    this.emit("revision.activated");
    return this.state;
  }

  async restoreRevision(input: { sessionId: string; revisionId: string }): Promise<{ revisionId: string; state: CadSessionState }> {
    const state = this.requireState();
    const sourceRevision = state.activeRevision;
    const revisionId = "revision-restored";
    const now = new Date().toISOString();
    const restoredRevision = {
      ...sourceRevision!,
      id: revisionId,
      parentRevisionId: state.session.activeRevisionId,
      restoredFromRevisionId: input.revisionId,
      createdAt: now,
      artifactCount: 0,
      artifacts: [],
      userEvents: [],
      runLinks: []
    };
    this.state = {
      ...state,
      session: {
        ...state.session,
        activeRevisionId: revisionId,
        revisions: [...state.session.revisions, restoredRevision]
      },
      activeRevision: restoredRevision
    };
    this.emit("revision.restored");
    return { revisionId, state: this.state };
  }

  async renderPreview(): Promise<{ state: CadSessionState }> {
    return { state: this.requireState() };
  }

  async persistRuntimeArtifact(): Promise<{ artifact: CadArtifact; state: CadSessionState }> {
    const state = this.requireState();
    return { artifact: state.activeRevision!.artifacts[0], state };
  }

  async updateParameters(): Promise<CadSessionState> {
    return this.requireState();
  }

  async createAgentRun(input: { sessionId: string; prompt: string; revisionId?: string; retryOfRunId?: string }): Promise<CreateAgentRunResult> {
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
      inputRevisionId: input.revisionId,
      externalAgent: "contract-test",
      status: "queued" as const,
      prompt: input.prompt,
      createdAt: now,
      updatedAt: now,
      recoveryStatus: "none" as const
    };
    const event = {
      id: "tauri-run-event-1",
      sessionId: input.sessionId,
      runId: run.id,
      revisionId: input.revisionId,
      type: "agent.run.created" as const,
      sequence: 1,
      createdAt: now,
      payload: { prompt: input.prompt, retryOfRunId: input.retryOfRunId }
    };
    this.state = {
      ...state,
      conversation: [...state.conversation, message],
      agentRuns: [...state.agentRuns, run],
      agentRunEvents: [...state.agentRunEvents, event]
    };
    this.emit("agent.run.created");
    return { message, run, state: this.state };
  }

  async startNewAgentConversation(sessionId: string) {
    const state = this.requireState();
    const now = new Date().toISOString();
    const activeThread = {
      id: "agent-thread-1",
      sessionId,
      plane: "modeling" as const,
      ownerId: sessionId,
      externalAgent: "codex",
      externalThreadId: "contract-thread-1",
      status: "ready" as const,
      connectionGeneration: 1,
      createdAt: now,
      updatedAt: now
    };
    this.state = { ...state, agentThreads: [activeThread] };
    return { activeThread, state: this.state };
  }

  async getAgentSessionDiagnostics(sessionId: string) {
    const state = this.requireState();
    return {
      sessionId,
      archived: false,
      threads: state.agentThreads.map((thread) => ({ thread, runs: [] })),
      unboundRuns: [],
      transportEventCount: 0
    };
  }

  async cleanupAgentTransportEvents() {
    return { deletedCount: 0, deletedEventIds: [] };
  }

  async cancelAgentRun(): Promise<{ run: never; state: CadSessionState }> {
    throw new Error("No cancellable run in contract fixture.");
  }

  async exportArtifact(): Promise<{ state: CadSessionState }> {
    return { state: this.requireState() };
  }

  async openArtifact() {
    return {
      artifact: this.requireState().activeRevision!.artifacts[0],
      path: "/tmp/cadastrophe-artifact.stl"
    };
  }

  async revealArtifact() {
    return {
      artifact: this.requireState().activeRevision!.artifacts[0],
      path: "/tmp/cadastrophe-artifact.stl",
      revealed: false
    };
  }

  async deleteArtifact(input: { sessionId: string; artifactId: string }) {
    const state = this.requireState();
    this.state = {
      ...state,
      activeRevision: state.activeRevision
        ? {
            ...state.activeRevision,
            artifactCount: state.activeRevision.artifacts.filter((artifact) => artifact.id !== input.artifactId).length,
            artifacts: state.activeRevision.artifacts.filter((artifact) => artifact.id !== input.artifactId)
          }
        : undefined
    };
    this.emit("artifact.deleted");
    return { artifactId: input.artifactId, state: this.state };
  }

  async verifyArtifactFiles() {
    return {
      checkedCount: this.requireState().activeRevision?.artifacts.length ?? 0,
      missingArtifactIds: [],
      hashMismatchArtifactIds: [],
      sizeMismatchArtifactIds: [],
      corruptMetadataArtifactIds: [],
      invalidPathArtifactIds: [],
      orphanPaths: [],
      diagnostics: [],
      state: this.requireState()
    };
  }

  async readPreviewMesh() {
    return { vertices: [], normals: [], indices: [] };
  }

  subscribeSession(
    sessionId: string,
    handlers: {
      onStatus: (status: "connecting" | "connected" | "disconnected") => void;
      onSnapshot: (state: CadSessionState) => void;
      onStream: (event: CadAgentStreamEvent) => void;
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
    if (!this.state) throw new Error("Contract session has not been created.");
    return this.state;
  }
}

function sampleState(title: string): CadSessionState {
  const now = new Date().toISOString();
  const artifact = {
    id: "artifact-1",
    revisionId: "revision-1",
    revisionHash: "a".repeat(64),
    kind: "stl" as const,
    format: "stl",
    uri: "artifacts/session-1/revision-1/artifact-1.stl",
    bytes: 128,
    createdAt: now,
    metadata: { source: "contract-fixture" }
  };
  return {
    session: {
      id: "session-1",
      createdAt: now,
      updatedAt: now,
      connectedUiClients: 0,
      title,
      titleSource: "user",
      activeRevisionId: "revision-1",
      selectedRuntime: "openscad-wasm",
      status: "idle",
      revisions: [
        {
          id: "revision-1",
          sourceHash: "sha256-source-fixture",
          sourceLanguage: "openscad",
          createdAt: now,
          diagnostics: { ok: true, elapsedMs: 0, items: [] },
          artifactCount: 1,
          runLinks: []
        }
      ]
    },
    activeRevision: {
      id: "revision-1",
      sessionId: "session-1",
      sourceHash: "sha256-source-fixture",
      sourceLanguage: "openscad",
      source: "cube([1, 1, 1]);",
      parameters: [],
      createdAt: now,
      diagnostics: { ok: true, elapsedMs: 0, items: [] },
      artifactCount: 1,
      artifacts: [artifact],
      userEvents: [],
      runLinks: []
    },
    messages: [],
    conversation: [],
    agentThreads: [],
    agentRuns: [],
    agentRunEvents: [],
    validationEvaluations: [
      {
        id: "validation-evaluation-1",
        sessionId: "session-1",
        runId: "workflow-run-1",
        revisionId: "revision-1",
        artifactId: "artifact-1",
        kind: "vlm",
        attempt: 1,
        status: "queued",
        inputContract: {
          contractType: "cadastrophe.vlm_evaluation_input.v1"
        },
        passThreshold: 0.8,
        createdAt: now
      }
    ],
    validationBatches: [{
      id: "validation-batch-1",
      sessionId: "session-1",
      runId: "workflow-run-1",
      revisionId: "revision-1",
      artifactId: "artifact-1",
      attempt: 1,
      status: "queued",
      createdAt: now
    }],
    validationChecks: (["structural", "dfm", "vlm"] as const).map((kind) => ({
      id: `validation-check-${kind}`,
      batchId: "validation-batch-1",
      sessionId: "session-1",
      kind,
      status: "queued",
      inputContract: { contractType: `cadastrophe.${kind}_input.v1` },
      createdAt: now
    })),
    workflow: {
      plans: [
        {
          runId: "workflow-run-1",
          revisionId: "revision-1",
          sourceLanguage: "openscad",
          createdAt: now,
          plan: {
            schemaVersion: "cad_model_plan.v1",
            summary: "Contract fixture bracket.",
            mainComponent: {
              name: "contract_bracket",
              purpose: "Exercise frontend workflow protocol."
            },
            supportingComponents: [],
            expectedAspectRatio: { x: 2, y: 1, z: 1, tolerance: 0.25 },
            sourceLanguage: "openscad",
            runtimeConstraints: {
              runtime: "openscad-wasm",
              mainComponentAnnotation: "contract_bracket"
            }
          }
        }
      ],
      outerIterations: [
        {
          id: "workflow-outer-1",
          runId: "workflow-run-1",
          iteration: 1,
          revisionId: "revision-1",
          structuralReport: {
            contractType: "cadastrophe.structural_report.v1",
            passed: false
          },
          failureReport: {
            contractType: "cadastrophe.failure_report.v1",
            reason: "structural_anchor_failed",
            summary: "Fixture structural failure."
          },
          passed: false,
          createdAt: now
        }
      ],
      pendingVlm: [
        {
          runId: "workflow-run-1",
          artifactId: "artifact-1",
          contract: {
            contractType: "cadastrophe.vlm_judge.v1",
            runId: "workflow-run-1",
            artifactId: "artifact-1"
          },
          passThreshold: 0.8,
          createdAt: now
        }
      ]
    }
  };
}

function sessionListItem(state: CadSessionState): CadSessionListItem {
  const activeRevision = state.session.revisions.find((revision) => revision.id === state.session.activeRevisionId);
  return {
    id: state.session.id,
    createdAt: state.session.createdAt,
    updatedAt: state.session.updatedAt,
    lastViewedAt: state.session.lastViewedAt,
    title: state.session.title,
    titleSource: state.session.titleSource,
    activeRevisionId: state.session.activeRevisionId,
    activeRevision,
    selectedRuntime: state.session.selectedRuntime,
    status: state.session.status,
    archived: Boolean(state.session.archivedAt),
    archivedAt: state.session.archivedAt,
    revisionCount: state.session.revisions.length,
    artifactCount: state.session.revisions.reduce((sum, revision) => sum + revision.artifactCount, 0)
  };
}

function assertSessionListShape(session: CadSessionListItem | undefined) {
  assert.ok(session);
  assert.equal(session.title, "Transport contract");
  assert.equal(typeof session.updatedAt, "string");
  assert.equal(session.archived, false);
  assert.equal(session.activeRevision?.id, session.activeRevisionId);
  assert.equal(typeof session.activeRevision?.sourceHash, "string");
}
