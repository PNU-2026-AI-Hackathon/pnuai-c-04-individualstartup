export type CadRuntimeKind = "openscad-wasm";

export type CadSourceLanguage =
  | "openscad"
  | "cadquery"
  | "freecad-python"
  | "cadastrophe-ir";

export type CadSessionStatus =
  | "idle"
  | "rendering"
  | "failed";

export type CadSessionTitleSource =
  | "agent"
  | "user"
  | "system";

export type CadArtifactKind = "preview-mesh" | "stl" | "metadata" | "render-image";

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
  | "agent.run.failed"
  | "agent.run.cancelled";

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
  deletedAt?: string;
  missingAt?: string;
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

export interface PersistRuntimeArtifactInput {
  sessionId: string;
  revisionId: string;
  kind: CadArtifactKind;
  format: string;
  contentsBase64: string;
  diagnostics: CadDiagnostics;
  metadata: Record<string, unknown>;
}

export interface PersistRuntimeArtifactResult {
  artifact: CadArtifact;
  state: CadSessionState;
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
  inputRevisionId?: string;
  outputRevisionId?: string;
  status: CadAgentRunStatus;
  prompt: string;
  createdAt: string;
  updatedAt: string;
  startedAt?: string;
  completedAt?: string;
  error?: string;
  activeStep?: string;
  externalAgent?: string;
  externalThreadId?: string;
  externalTurnId?: string;
}

export interface CadAgentRunEvent {
  id: string;
  sessionId: string;
  runId: string;
  revisionId?: string;
  type: CadAgentRunEventType;
  sequence: number;
  createdAt: string;
  payload: Record<string, unknown>;
  metadata?: Record<string, unknown>;
}

export interface CadModelPlanComponent {
  name: string;
  purpose: string;
  requiredFeatures?: string[];
}

export interface CadModelAspectRatio {
  x: number;
  y: number;
  z: number;
  tolerance: number;
}

export interface CadModelRuntimeConstraints {
  runtime: CadRuntimeKind;
  requiredFeatures?: string[];
  forbiddenFeatures?: string[];
  mainComponentAnnotation?: string;
}

export interface CadModelPlan {
  schemaVersion: string;
  summary: string;
  mainComponent: CadModelPlanComponent;
  supportingComponents: CadModelPlanComponent[];
  expectedAspectRatio: CadModelAspectRatio;
  sourceLanguage: CadSourceLanguage;
  runtimeConstraints: CadModelRuntimeConstraints;
}

export interface CadWorkflowPlan {
  runId: string;
  revisionId?: string;
  plan: CadModelPlan;
  sourceLanguage: CadSourceLanguage;
  createdAt: string;
}

export interface CadWorkflowOuterIteration {
  id: string;
  runId: string;
  iteration: number;
  revisionId?: string;
  structuralReport: Record<string, unknown>;
  vlmReport?: Record<string, unknown>;
  failureReport?: Record<string, unknown>;
  passed: boolean;
  createdAt: string;
}

export interface CadWorkflowPendingVlm {
  runId: string;
  artifactId: string;
  contract: Record<string, unknown>;
  passThreshold: number;
  createdAt: string;
}

export interface CadWorkflowState {
  plans: CadWorkflowPlan[];
  outerIterations: CadWorkflowOuterIteration[];
  pendingVlm: CadWorkflowPendingVlm[];
}

export interface CadRevisionRunLink {
  runId: string;
  role: "input" | "output" | string;
  status: CadAgentRunStatus;
  updatedAt: string;
}

export interface CadRevisionSummary {
  id: string;
  sourceHash: string;
  parentRevisionId?: string;
  restoredFromRevisionId?: string;
  sourceLanguage: CadSourceLanguage;
  createdAt: string;
  diagnostics: CadDiagnostics;
  artifactCount: number;
  runLinks: CadRevisionRunLink[];
}

export interface CadRevision extends CadRevisionSummary {
  sessionId: string;
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
  titleSource: CadSessionTitleSource;
  activeRevisionId?: string;
  selectedRuntime: CadRuntimeKind;
  status: CadSessionStatus;
  recoveryDiagnostics?: CadDiagnostic[];
  archivedAt?: string;
  deletedAt?: string;
  revisions: CadRevisionSummary[];
}

export interface CadSessionState {
  session: CadSession;
  activeRevision?: CadRevision;
  messages: CadUserMessage[];
  conversation: CadConversationMessage[];
  agentRuns: CadAgentRun[];
  agentRunEvents: CadAgentRunEvent[];
  workflow: CadWorkflowState;
}

export interface CadBridgeEvent {
  id: string;
  type:
    | "session.created"
    | "session.updated"
    | "revision.created"
    | "revision.activated"
    | "revision.restored"
    | "preview.rendered"
    | "message.created"
    | "artifact.exported"
    | "artifact.deleted"
    | "artifact.verified"
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

export interface BootCadSessionResult {
  sessionId: string;
  uiUrl: string;
  state: CadSessionState;
  isFirstRun: boolean;
  createdSession: boolean;
  shouldUseExampleSession: boolean;
  shouldAutoRender: boolean;
}

export interface CurrentCadSessionResult {
  sessionId?: string;
  uiUrl?: string;
  state?: CadSessionState;
}

export interface ListCadSessionsInput {
  includeArchived?: boolean;
  query?: string;
}

export interface CadSessionListItem {
  id: string;
  createdAt: string;
  updatedAt: string;
  lastViewedAt?: string;
  title?: string;
  titleSource: CadSessionTitleSource;
  activeRevisionId?: string;
  activeRevision?: CadRevisionSummary;
  selectedRuntime: CadRuntimeKind;
  status: CadSessionStatus;
  archived: boolean;
  archivedAt?: string;
  revisionCount: number;
  artifactCount: number;
}

export interface ListCadSessionsResult {
  sessions: CadSessionListItem[];
  searchFields: string[];
}

export interface RenameCadSessionInput {
  sessionId: string;
  title: string;
}

export interface ArchiveCadSessionInput {
  sessionId: string;
  archived?: boolean;
}

export interface DuplicateCadSessionInput {
  sessionId: string;
  title?: string;
}

export interface DeleteCadSessionInput {
  sessionId: string;
}

export interface DeleteCadSessionResult {
  sessionId: string;
  currentSessionId?: string;
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

export interface SetActiveRevisionInput {
  sessionId: string;
  revisionId: string;
}

export interface RestoreRevisionInput {
  sessionId: string;
  revisionId: string;
}

export interface RestoreRevisionResult {
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
  retryOfRunId?: string;
}

export interface CreateAgentRunResult {
  message: CadConversationMessage;
  run: CadAgentRun;
  state: CadSessionState;
}

export interface DeleteArtifactInput {
  sessionId: string;
  artifactId: string;
}

export interface DeleteArtifactResult {
  artifactId: string;
  state: CadSessionState;
}

export interface OpenArtifactResult {
  artifact: CadArtifact;
  path: string;
}

export interface RevealArtifactResult {
  artifact: CadArtifact;
  path: string;
  revealed: boolean;
}

export interface VerifyArtifactFilesResult {
  checkedCount: number;
  missingArtifactIds: string[];
  hashMismatchArtifactIds: string[];
  sizeMismatchArtifactIds: string[];
  corruptMetadataArtifactIds: string[];
  invalidPathArtifactIds: string[];
  orphanPaths: string[];
  diagnostics: CadDiagnostic[];
  state?: CadSessionState;
}

export interface CleanupOrphanArtifactsResult {
  checkedFileCount: number;
  orphanPaths: string[];
  deletedPaths: string[];
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
