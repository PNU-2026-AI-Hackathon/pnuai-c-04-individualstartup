export type CadRuntimeKind = "openscad-wasm" | "cadquery-local" | "freecad-local";

export type CadSourceLanguage =
  | "openscad"
  | "cadquery"
  | "freecad-python"
  | "cadastrophe-ir";

export type CadSessionStatus =
  | "idle"
  | "rendering"
  | "failed";

export type CadArtifactKind = "preview-mesh" | "stl" | "metadata";

export type CadUserMessageChannel = "web-ui";

export type CadConversationRole = "user" | "assistant" | "system" | "tool";

export type CadAgentRunStatus =
  | "queued"
  | "running"
  | "waiting_for_user"
  | "completed"
  | "failed"
  | "cancelled";

export type CadAgentRunEventType =
  | "agent.run.created"
  | "agent.run.updated"
  | "agent.message.created"
  | "agent.tool.started"
  | "agent.tool.completed"
  | "agent.run.completed"
  | "agent.run.failed";

export interface CadRuntimeCapabilities {
  kind: CadRuntimeKind;
  available: boolean;
  sourceLanguages: CadSourceLanguage[];
  previewFormats: string[];
  exportFormats: string[];
  limitations: string[];
}

export interface CadDiagnostic {
  severity: "info" | "warning" | "error";
  message: string;
  line?: number;
  column?: number;
}

export interface CadDiagnostics {
  ok: boolean;
  elapsedMs: number;
  items: CadDiagnostic[];
}

export interface CadParameter {
  name: string;
  value: number | string | boolean;
  type: "number" | "string" | "boolean";
  min?: number;
  max?: number;
  step?: number;
  label?: string;
}

export interface CadMesh {
  vertices: number[];
  normals: number[];
  indices: number[];
}

export interface CadArtifact {
  id: string;
  revisionId: string;
  kind: CadArtifactKind;
  format: string;
  uri: string;
  bytes?: number;
  createdAt: string;
  metadata?: Record<string, unknown>;
}

export interface CadPreviewResult {
  diagnostics: CadDiagnostics;
  mesh?: CadMesh;
  artifacts: CadArtifact[];
}

export interface CadExportResult {
  diagnostics: CadDiagnostics;
  artifact?: CadArtifact;
}

export interface CadBuildInput {
  sessionId: string;
  revisionId: string;
  sourceLanguage: CadSourceLanguage;
  source: string;
  parameters: CadParameter[];
}

export interface CadExportInput extends CadBuildInput {
  format: "stl" | "metadata";
}

export interface CadUserEvent {
  id: string;
  revisionId: string;
  type:
    | "decision.approved"
    | "decision.rejected"
    | "parameter.updated"
    | "message.created"
    | "export.requested"
    | "runtime.selected";
  createdAt: string;
  payload: Record<string, unknown>;
}

export interface CadUserMessage {
  id: string;
  sessionId: string;
  revisionId?: string;
  eventId?: string;
  channel: CadUserMessageChannel;
  message: string;
  createdAt: string;
}

export interface CadConversationMessage {
  id: string;
  sessionId: string;
  revisionId?: string;
  role: CadConversationRole;
  content: string;
  createdAt: string;
  runId?: string;
  metadata?: Record<string, unknown>;
}

export interface CadAgentRun {
  id: string;
  sessionId: string;
  status: CadAgentRunStatus;
  prompt: string;
  createdAt: string;
  updatedAt: string;
  startedAt?: string;
  completedAt?: string;
  error?: string;
  activeStep?: string;
}

export interface CadRevisionSummary {
  id: string;
  sourceLanguage: CadSourceLanguage;
  createdAt: string;
  diagnostics: CadDiagnostics;
  artifactCount: number;
}

export interface CadRevision extends CadRevisionSummary {
  sessionId: string;
  parentRevisionId?: string;
  source: string;
  parameters: CadParameter[];
  artifacts: CadArtifact[];
  userEvents: CadUserEvent[];
}

export interface CadSession {
  id: string;
  createdAt: string;
  updatedAt: string;
  lastViewedAt?: string;
  connectedUiClients: number;
  title?: string;
  activeRevisionId?: string;
  selectedRuntime: CadRuntimeKind;
  status: CadSessionStatus;
  revisions: CadRevisionSummary[];
}

export interface CadSessionState {
  session: CadSession;
  activeRevision?: CadRevision;
  messages: CadUserMessage[];
  conversation: CadConversationMessage[];
  agentRuns: CadAgentRun[];
}

export interface CadBridgeEvent {
  id: string;
  type:
    | "session.created"
    | "session.updated"
    | "revision.created"
    | "preview.rendered"
    | "message.created"
    | "artifact.exported"
    | CadAgentRunEventType;
  sessionId: string;
  createdAt: string;
  state: CadSessionState;
}

export interface CreateCadSessionInput {
  title?: string;
  selectedRuntime?: CadRuntimeKind;
}

export interface CreateCadSessionResult {
  sessionId: string;
  uiUrl: string;
  state: CadSessionState;
}

export interface CurrentCadSessionResult {
  sessionId?: string;
  uiUrl?: string;
  state?: CadSessionState;
}

export interface UpdateModelSourceInput {
  sessionId: string;
  sourceLanguage: CadSourceLanguage;
  source: string;
  parentRevisionId?: string;
  parameters?: CadParameter[];
}

export interface UpdateModelSourceResult {
  revisionId: string;
  state: CadSessionState;
}

export interface RenderPreviewInput {
  sessionId: string;
  revisionId?: string;
}

export interface PostUserMessageInput {
  sessionId: string;
  revisionId?: string;
  message: string;
}

export interface CreateAgentRunInput {
  sessionId: string;
  prompt: string;
  revisionId?: string;
}

export interface CreateAgentRunResult {
  message: CadConversationMessage;
  run: CadAgentRun;
  state: CadSessionState;
}

export interface WaitForUserMessageInput {
  sessionId: string;
  afterMessageId?: string;
  timeoutMs?: number;
}

export interface ExportArtifactInput {
  sessionId: string;
  revisionId?: string;
  format: "stl" | "metadata";
}
