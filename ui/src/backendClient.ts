import type {
  BootCadSessionResult,
  CadAgentRun,
  CadArtifact,
  CadBridgeEvent,
  CadMesh,
  CadParameter,
  PersistRuntimeArtifactInput,
  PersistRuntimeArtifactResult,
  CadSessionState,
  CreateAgentRunResult,
  CreateCadSessionResult,
  CurrentCadSessionResult,
  DeleteArtifactResult,
  DeleteCadSessionResult,
  ListCadSessionsInput,
  ListCadSessionsResult,
  OpenArtifactResult,
  RevealArtifactResult,
  RestoreRevisionResult,
  UpdateModelSourceInput,
  UpdateModelSourceResult,
  VerifyArtifactFilesResult
} from "./protocol";

export type ConnectionStatus = "connecting" | "connected" | "disconnected";

export interface CadBackendClient {
  createSession(input: { title?: string }): Promise<CreateCadSessionResult>;
  bootSession(): Promise<BootCadSessionResult>;
  getCurrentSession(): Promise<CurrentCadSessionResult>;
  getSessionState(sessionId: string): Promise<CadSessionState>;
  markSessionViewed(sessionId: string): Promise<CadSessionState>;
  listSessions(input?: ListCadSessionsInput): Promise<ListCadSessionsResult>;
  renameSession(input: { sessionId: string; title: string }): Promise<CadSessionState>;
  archiveSession(input: { sessionId: string; archived?: boolean }): Promise<CadSessionState>;
  deleteSession(sessionId: string): Promise<DeleteCadSessionResult>;
  duplicateSession(input: { sessionId: string; title?: string }): Promise<CreateCadSessionResult>;
  updateModelSource(input: UpdateModelSourceInput): Promise<UpdateModelSourceResult>;
  setActiveRevision(input: { sessionId: string; revisionId: string }): Promise<CadSessionState>;
  restoreRevision(input: { sessionId: string; revisionId: string }): Promise<RestoreRevisionResult>;
  renderPreview(input: { sessionId: string; revisionId?: string }): Promise<{ state: CadSessionState }>;
  persistRuntimeArtifact(input: PersistRuntimeArtifactInput): Promise<PersistRuntimeArtifactResult>;
  updateParameters(input: {
    sessionId: string;
    values: Record<string, CadParameter["value"]>;
  }): Promise<CadSessionState>;
  createAgentRun(input: { sessionId: string; prompt: string; revisionId?: string; retryOfRunId?: string }): Promise<CreateAgentRunResult>;
  cancelAgentRun(input: { sessionId: string; runId: string }): Promise<{ run: CadAgentRun; state: CadSessionState }>;
  exportArtifact(input: { sessionId: string; revisionId?: string; format: "stl" | "metadata" }): Promise<{ state: CadSessionState }>;
  openArtifact(artifactId: string): Promise<OpenArtifactResult>;
  revealArtifact(artifactId: string): Promise<RevealArtifactResult>;
  deleteArtifact(input: { sessionId: string; artifactId: string }): Promise<DeleteArtifactResult>;
  verifyArtifactFiles(input: { sessionId?: string }): Promise<VerifyArtifactFilesResult>;
  readPreviewMesh(artifact: CadArtifact): Promise<CadMesh>;
  subscribeSession(
    sessionId: string,
    handlers: {
      onStatus: (status: ConnectionStatus) => void;
      onSnapshot: (state: CadSessionState) => void;
      onError: (error: unknown) => void;
    }
  ): () => void;
}

export function createCadBackendClient(): CadBackendClient {
  return new TauriCadBackendClient();
}

export class TauriCadBackendClient implements CadBackendClient {
  createSession(input: { title?: string }): Promise<CreateCadSessionResult> {
    return invokeCommand("create_session", { input });
  }

  bootSession(): Promise<BootCadSessionResult> {
    return invokeCommand("boot_session");
  }

  getCurrentSession(): Promise<CurrentCadSessionResult> {
    return invokeCommand("get_current_session");
  }

  getSessionState(sessionId: string): Promise<CadSessionState> {
    return invokeCommand("get_session_state", { sessionId });
  }

  markSessionViewed(sessionId: string): Promise<CadSessionState> {
    return invokeCommand("mark_session_viewed", { sessionId });
  }

  listSessions(input: ListCadSessionsInput = {}): Promise<ListCadSessionsResult> {
    return invokeCommand("list_sessions", { input });
  }

  renameSession(input: { sessionId: string; title: string }): Promise<CadSessionState> {
    return invokeCommand("rename_session", { input });
  }

  archiveSession(input: { sessionId: string; archived?: boolean }): Promise<CadSessionState> {
    return invokeCommand("archive_session", { input });
  }

  deleteSession(sessionId: string): Promise<DeleteCadSessionResult> {
    return invokeCommand("delete_session", { input: { sessionId } });
  }

  duplicateSession(input: { sessionId: string; title?: string }): Promise<CreateCadSessionResult> {
    return invokeCommand("duplicate_session", { input });
  }

  updateModelSource(input: UpdateModelSourceInput): Promise<UpdateModelSourceResult> {
    return invokeCommand("update_model_source", { input });
  }

  setActiveRevision(input: { sessionId: string; revisionId: string }): Promise<CadSessionState> {
    return invokeCommand("set_active_revision", { input });
  }

  restoreRevision(input: { sessionId: string; revisionId: string }): Promise<RestoreRevisionResult> {
    return invokeCommand("restore_revision", { input });
  }

  renderPreview(input: { sessionId: string; revisionId?: string }): Promise<{ state: CadSessionState }> {
    return invokeCommand("render_preview", { input });
  }

  persistRuntimeArtifact(input: PersistRuntimeArtifactInput): Promise<PersistRuntimeArtifactResult> {
    return invokeCommand("persist_runtime_artifact", { input });
  }

  updateParameters(input: {
    sessionId: string;
    values: Record<string, CadParameter["value"]>;
  }): Promise<CadSessionState> {
    return invokeCommand("update_parameters", { input });
  }

  createAgentRun(input: { sessionId: string; prompt: string; revisionId?: string; retryOfRunId?: string }): Promise<CreateAgentRunResult> {
    return invokeCommand("create_agent_run", { input });
  }

  cancelAgentRun(input: { sessionId: string; runId: string }): Promise<{ run: CadAgentRun; state: CadSessionState }> {
    return invokeCommand("cancel_agent_run", input);
  }

  exportArtifact(input: {
    sessionId: string;
    revisionId?: string;
    format: "stl" | "metadata";
  }): Promise<{ state: CadSessionState }> {
    return invokeCommand("export_artifact", { input });
  }

  async readPreviewMesh(artifact: CadArtifact): Promise<CadMesh> {
    const contents = await invokeCommand<string>("read_artifact", { artifactId: artifact.id });
    return JSON.parse(contents) as CadMesh;
  }

  openArtifact(artifactId: string): Promise<OpenArtifactResult> {
    return invokeCommand("open_artifact", { artifactId });
  }

  revealArtifact(artifactId: string): Promise<RevealArtifactResult> {
    return invokeCommand("reveal_artifact", { artifactId });
  }

  deleteArtifact(input: { sessionId: string; artifactId: string }): Promise<DeleteArtifactResult> {
    return invokeCommand("delete_artifact", { input });
  }

  verifyArtifactFiles(input: { sessionId?: string }): Promise<VerifyArtifactFilesResult> {
    return invokeCommand("verify_artifact_files", { input });
  }

  subscribeSession(
    sessionId: string,
    handlers: {
      onStatus: (status: ConnectionStatus) => void;
      onSnapshot: (state: CadSessionState) => void;
      onError: (error: unknown) => void;
    }
  ): () => void {
    let cleanup: (() => void) | undefined;
    let closedByCaller = false;
    handlers.onStatus("connecting");
    import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<CadBridgeEvent>("cad_bridge_event", (event) => {
          if (event.payload.sessionId === sessionId) {
            handlers.onSnapshot(event.payload.state);
          }
        })
      )
      .then((unlisten) => {
        cleanup = unlisten;
        if (!closedByCaller) handlers.onStatus("connected");
      })
      .catch((error) => {
        handlers.onStatus("disconnected");
        handlers.onError(error);
      });

    return () => {
      closedByCaller = true;
      cleanup?.();
      handlers.onStatus("disconnected");
    };
  }
}

async function invokeCommand<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(command, args);
}
