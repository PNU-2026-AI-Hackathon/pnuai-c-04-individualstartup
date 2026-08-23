import type {
  BootCadSessionResult,
  CadAgentRun,
  CadAgentSessionDiagnostics,
  CadAgentTransportCleanupInput,
  CadAgentTransportCleanupResult,
  CadArtifact,
  CadAgentStreamEvent,
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
  ExportArtifactFileResult,
  ListCadSessionsInput,
  ListCadSessionsResult,
  OpenArtifactResult,
  RevealArtifactResult,
  RestoreRevisionResult,
  StartNewAgentConversationResult,
  UpdateModelSourceInput,
  UpdateModelSourceResult,
  VerifyArtifactFilesResult
} from "./protocol";

export type ConnectionStatus = "connecting" | "connected" | "disconnected";

export interface PrusaSlicerValidation {
  path: string;
  version: string;
}

export interface DfmProfileValidation {
  hash: string;
  keySettings: Record<string, string>;
}

export interface DfmProfileDocument extends DfmProfileValidation {
  contents: string;
}

export interface DfmSettings {
  prusaslicerExecutable: string | null;
  executableValidation: PrusaSlicerValidation | null;
  profile: DfmProfileDocument;
}

export interface DfmSettingsBackendClient {
  getDfmSettings(): Promise<DfmSettings>;
  validatePrusaSlicerExecutable(input: { path: string }): Promise<PrusaSlicerValidation>;
  savePrusaSlicerExecutable(input: { path: string }): Promise<PrusaSlicerValidation>;
  validateDfmProfile(input: { contents: string }): Promise<DfmProfileValidation>;
  saveDfmProfile(input: { contents: string }): Promise<DfmProfileDocument>;
  importDfmProfile(input: { path: string }): Promise<{ contents: string; sourcePath: string }>;
  exportDfmProfile(input: { path: string; contents: string }): Promise<{ path: string }>;
  restoreDefaultDfmProfile(): Promise<DfmProfileDocument>;
}

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
  startNewAgentConversation(sessionId: string): Promise<StartNewAgentConversationResult>;
  getAgentSessionDiagnostics(sessionId: string): Promise<CadAgentSessionDiagnostics>;
  cleanupAgentTransportEvents(input: CadAgentTransportCleanupInput): Promise<CadAgentTransportCleanupResult>;
  cancelAgentRun(input: { sessionId: string; runId: string }): Promise<{ run: CadAgentRun; state: CadSessionState }>;
  exportArtifact(input: { sessionId: string; revisionId?: string; format: "stl" | "metadata" }): Promise<{ state: CadSessionState }>;
  exportArtifactFile(input: { artifactId: string; path: string }): Promise<ExportArtifactFileResult>;
  openArtifact(artifactId: string): Promise<OpenArtifactResult>;
  revealArtifact(artifactId: string): Promise<RevealArtifactResult>;
  deleteArtifact(input: { sessionId: string; artifactId: string }): Promise<DeleteArtifactResult>;
  verifyArtifactFiles(input: { sessionId?: string }): Promise<VerifyArtifactFilesResult>;
  readPreviewMesh(artifact: CadArtifact): Promise<CadMesh>;
  readGcode(artifact: CadArtifact): Promise<string>;
  subscribeSession(
    sessionId: string,
    handlers: {
      onStatus: (status: ConnectionStatus) => void;
      onSnapshot: (state: CadSessionState) => void;
      onStream: (event: CadAgentStreamEvent) => void;
      onError: (error: unknown) => void;
    }
  ): () => void;
}

export function createCadBackendClient(): CadBackendClient & DfmSettingsBackendClient {
  return new TauriCadBackendClient();
}

export class TauriCadBackendClient implements CadBackendClient, DfmSettingsBackendClient {
  getDfmSettings(): Promise<DfmSettings> {
    return invokeCommand("get_dfm_settings");
  }

  validatePrusaSlicerExecutable(input: { path: string }): Promise<PrusaSlicerValidation> {
    return invokeCommand("validate_prusaslicer_executable", { input });
  }

  savePrusaSlicerExecutable(input: { path: string }): Promise<PrusaSlicerValidation> {
    return invokeCommand("save_prusaslicer_executable", { input });
  }

  validateDfmProfile(input: { contents: string }): Promise<DfmProfileValidation> {
    return invokeCommand("validate_dfm_profile", { input });
  }

  saveDfmProfile(input: { contents: string }): Promise<DfmProfileDocument> {
    return invokeCommand("save_dfm_profile", { input });
  }

  importDfmProfile(input: { path: string }): Promise<{ contents: string; sourcePath: string }> {
    return invokeCommand("import_dfm_profile", { input });
  }

  exportDfmProfile(input: { path: string; contents: string }): Promise<{ path: string }> {
    return invokeCommand("export_dfm_profile", { input });
  }

  restoreDefaultDfmProfile(): Promise<DfmProfileDocument> {
    return invokeCommand("restore_default_dfm_profile");
  }

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

  startNewAgentConversation(sessionId: string): Promise<StartNewAgentConversationResult> {
    return invokeCommand("start_new_agent_conversation", { input: { sessionId } });
  }

  getAgentSessionDiagnostics(sessionId: string): Promise<CadAgentSessionDiagnostics> {
    return invokeCommand("get_agent_session_diagnostics", { sessionId });
  }

  cleanupAgentTransportEvents(input: CadAgentTransportCleanupInput): Promise<CadAgentTransportCleanupResult> {
    return invokeCommand("cleanup_agent_transport_events", { input });
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

  exportArtifactFile(input: { artifactId: string; path: string }): Promise<ExportArtifactFileResult> {
    return invokeCommand("export_artifact_file", { input });
  }

  async readPreviewMesh(artifact: CadArtifact): Promise<CadMesh> {
    const contents = await invokeCommand<string>("read_artifact", { artifactId: artifact.id });
    return JSON.parse(contents) as CadMesh;
  }

  async readGcode(artifact: CadArtifact): Promise<string> {
    if (artifact.kind !== "gcode" || artifact.format !== "gcode") {
      throw new Error(`Artifact ${artifact.id} is not G-code.`);
    }
    const contents = await invokeCommand<string>("read_artifact", { artifactId: artifact.id });
    if (!contents.trim()) {
      throw new Error(`G-code artifact ${artifact.id} is empty.`);
    }
    return contents;
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
      onStream: (event: CadAgentStreamEvent) => void;
      onError: (error: unknown) => void;
    }
  ): () => void {
    let cleanup: (() => void) | undefined;
    let closedByCaller = false;
    handlers.onStatus("connecting");
    import("@tauri-apps/api/event")
      .then(async ({ listen }) => {
        const unlistenSnapshot = await listen<CadBridgeEvent>("cad_bridge_event", (event) => {
          if (event.payload.sessionId === sessionId) {
            handlers.onSnapshot(event.payload.state);
          }
        });
        let unlistenStream: (() => void) | undefined;
        try {
          unlistenStream = await listen<CadAgentStreamEvent>("agent_stream_event", (event) => {
            if (event.payload.sessionId === sessionId) {
              handlers.onStream(event.payload);
            }
          });
        } catch (error) {
          unlistenSnapshot();
          throw error;
        }
        cleanup = () => {
          unlistenSnapshot();
          unlistenStream();
        };
        if (closedByCaller) {
          cleanup();
          return;
        }
        handlers.onStatus("connected");
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
