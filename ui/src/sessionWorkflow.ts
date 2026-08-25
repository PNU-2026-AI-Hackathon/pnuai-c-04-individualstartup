import type { CadBackendClient } from "./backendClient";
import type {
  CadRevision,
  CadSessionState,
  CreateCadSessionResult,
  UpdateModelSourceResult
} from "./protocol";

interface DuplicateSessionWorkflowInput {
  backend: Pick<CadBackendClient, "duplicateSession" | "markSessionViewed">;
  sessionId: string;
  applySessionSnapshot: (state: CadSessionState) => void;
  renderRevision: (sessionId: string, revision?: CadRevision) => Promise<void>;
}

export async function duplicateSessionWithPreview({
  backend,
  sessionId,
  applySessionSnapshot,
  renderRevision
}: DuplicateSessionWorkflowInput): Promise<CreateCadSessionResult> {
  const duplicated = await backend.duplicateSession({ sessionId });
  applySessionSnapshot(duplicated.state);
  await backend.markSessionViewed(duplicated.sessionId);
  await renderRevision(duplicated.sessionId, duplicated.state.activeRevision);
  return duplicated;
}

interface SaveSourceRevisionWorkflowInput {
  backend: Pick<CadBackendClient, "updateModelSource">;
  sessionId: string;
  source: string;
  parentRevisionId?: string;
  applySessionSnapshot: (state: CadSessionState) => void;
  renderRevision: (sessionId: string, revision: CadRevision) => Promise<void>;
}

export async function saveSourceRevisionWithPreview({
  backend,
  sessionId,
  source,
  parentRevisionId,
  applySessionSnapshot,
  renderRevision
}: SaveSourceRevisionWorkflowInput): Promise<UpdateModelSourceResult> {
  const result = await backend.updateModelSource({
    sessionId,
    sourceLanguage: "openscad",
    source,
    parentRevisionId
  });
  const revision = result.state.activeRevision;
  if (!revision || revision.id !== result.revisionId) {
    throw new Error(`Saved source revision ${result.revisionId} is not active in the returned session state.`);
  }
  applySessionSnapshot(result.state);
  await renderRevision(sessionId, revision);
  return result;
}
