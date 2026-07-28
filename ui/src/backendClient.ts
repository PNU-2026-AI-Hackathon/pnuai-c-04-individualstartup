import type {
  CadAgentRun,
  CadArtifact,
  CadBridgeEvent,
  CadMesh,
  CadParameter,
  CadSessionState,
  CreateAgentRunResult,
  CreateCadSessionResult,
  CurrentCadSessionResult,
  UpdateModelSourceResult
} from "./protocol";

export type ConnectionStatus = "connecting" | "connected" | "disconnected";

export interface CadBackendClient {
  createSession(input: { title?: string }): Promise<CreateCadSessionResult>;
  getCurrentSession(): Promise<CurrentCadSessionResult>;
  getSessionState(sessionId: string): Promise<CadSessionState>;
  markSessionViewed(sessionId: string): Promise<CadSessionState>;
  updateModelSource(input: {
    sessionId: string;
    sourceLanguage: "openscad";
    source: string;
    parentRevisionId?: string;
  }): Promise<UpdateModelSourceResult>;
  renderPreview(input: { sessionId: string; revisionId?: string }): Promise<{ state: CadSessionState }>;
  updateParameters(input: {
    sessionId: string;
    values: Record<string, CadParameter["value"]>;
  }): Promise<CadSessionState>;
  createAgentRun(input: { sessionId: string; prompt: string; revisionId?: string }): Promise<CreateAgentRunResult>;
  cancelAgentRun(input: { sessionId: string; runId: string }): Promise<{ run: CadAgentRun; state: CadSessionState }>;
  exportArtifact(input: { sessionId: string; revisionId?: string; format: "stl" | "metadata" }): Promise<{ state: CadSessionState }>;
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

  getCurrentSession(): Promise<CurrentCadSessionResult> {
    return invokeCommand("get_current_session");
  }

  getSessionState(sessionId: string): Promise<CadSessionState> {
    return invokeCommand("get_session_state", { sessionId });
  }

  markSessionViewed(sessionId: string): Promise<CadSessionState> {
    return invokeCommand("mark_session_viewed", { sessionId });
  }

  updateModelSource(input: {
    sessionId: string;
    sourceLanguage: "openscad";
    source: string;
    parentRevisionId?: string;
  }): Promise<UpdateModelSourceResult> {
    return invokeCommand("update_model_source", { input });
  }

  renderPreview(input: { sessionId: string; revisionId?: string }): Promise<{ state: CadSessionState }> {
    return invokeCommand("render_preview", { input });
  }

  updateParameters(input: {
    sessionId: string;
    values: Record<string, CadParameter["value"]>;
  }): Promise<CadSessionState> {
    return invokeCommand("update_parameters", { input });
  }

  createAgentRun(input: { sessionId: string; prompt: string; revisionId?: string }): Promise<CreateAgentRunResult> {
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
