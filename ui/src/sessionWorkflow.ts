import type { CadBackendClient } from "./backendClient";
import type { CadRevision, CadSessionState, CreateCadSessionResult } from "./protocol";

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
