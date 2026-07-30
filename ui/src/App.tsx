import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { List } from "lucide-react";
import type {
  CadMesh,
  CadParameter,
  CadRevision,
  CadSessionListItem,
  CadSessionState,
  VerifyArtifactFilesResult
} from "./protocol";
import { createCadBackendClient, type ConnectionStatus } from "./backendClient";
import { ArtifactBrowser } from "./components/ArtifactBrowser";
import { isActiveRunStatus } from "./components/AgentWorkflow";
import { SessionLogs } from "./components/SessionLogs";
import { SessionBrowser } from "./components/SessionBrowser";
import { WorkspaceNav } from "./components/WorkspaceNav";
import { WorkspacePanel } from "./components/WorkspacePanel";
import {
  base64EncodeBytes,
  base64EncodeUtf8,
  errorMessage,
  parameterValues,
  replaceUrl,
  runtimeMetadata,
  sha256Hex
} from "./appUtils";
import {
  sessionIdFromUrl,
  sessionPathWithView,
  workspaceViewFromUrl,
  type WorkspaceView
} from "./navigation";
import { renderOpenScadInWorker, type OpenscadRenderResult, type OpenscadRuntimeState } from "./runtime/openscadRuntime";

export function App() {
  const backend = useMemo(() => createCadBackendClient(), []);
  const [state, setState] = useState<CadSessionState | null>(null);
  const [source, setSource] = useState("");
  const [sourceDirty, setSourceDirty] = useState(false);
  const [sourceConflict, setSourceConflict] = useState(false);
  const sourceDirtyRef = useRef(false);
  const sourceRevisionIdRef = useRef<string | undefined>(undefined);
  const latestSessionIdRef = useRef<string | undefined>(undefined);
  const latestSessionUpdatedAtRef = useRef<string | undefined>(undefined);
  const [mesh, setMesh] = useState<CadMesh | null>(null);
  const [runtimeState, setRuntimeState] = useState<OpenscadRuntimeState>("idle");
  const [agentPrompt, setAgentPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [openedArtifactPath, setOpenedArtifactPath] = useState<string | null>(null);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>("disconnected");
  const [view, setView] = useState<WorkspaceView>(() =>
    workspaceViewFromUrl(window.location.href, window.location.href)
  );
  const [sessionSearch, setSessionSearch] = useState("");
  const [sessionList, setSessionList] = useState<CadSessionListItem[]>([]);
  const locallyDeletedSessionIdsRef = useRef<Set<string>>(new Set());
  const [sessionSearchFields, setSessionSearchFields] = useState<string[]>([]);
  const [integrityResult, setIntegrityResult] = useState<VerifyArtifactFilesResult | null>(null);
  const previewCacheRef = useRef<
    | (OpenscadRenderResult & {
        revisionId: string;
      })
    | null
  >(null);

  const sessionId = useMemo(() => sessionIdFromUrl(window.location.href, window.location.href), []);
  const activeRevision = state?.activeRevision;
  const previewArtifact = activeRevision?.artifacts.find((artifact) => artifact.kind === "preview-mesh");
  const activeAgentRun = state?.agentRuns.find((run) => isActiveRunStatus(run.status));
  const sessionArchived = Boolean(state?.session.archivedAt);

  const applySessionSnapshot = useCallback(
    (nextState: CadSessionState, options: { forceSource?: boolean } = {}) => {
      if (
        latestSessionIdRef.current === nextState.session.id &&
        latestSessionUpdatedAtRef.current &&
        nextState.session.updatedAt < latestSessionUpdatedAtRef.current
      ) {
        return;
      }
      const nextRevisionId = nextState.session.activeRevisionId;
      latestSessionIdRef.current = nextState.session.id;
      latestSessionUpdatedAtRef.current = nextState.session.updatedAt;
      setState(nextState);
      if (options.forceSource || !sourceDirtyRef.current) {
        setSource(nextState.activeRevision?.source ?? "");
        setSourceDirty(false);
        setSourceConflict(false);
        sourceDirtyRef.current = false;
        sourceRevisionIdRef.current = nextRevisionId;
        return;
      }
      if (nextRevisionId !== sourceRevisionIdRef.current) {
        setSourceConflict(true);
      }
    },
    []
  );

  const loadSession = useCallback(
    async (targetSessionId: string, options: { forceSource?: boolean } = {}) => {
      const nextState = await backend.getSessionState(targetSessionId);
      applySessionSnapshot(nextState, options);
      return nextState;
    },
    [applySessionSnapshot, backend]
  );

  useEffect(() => {
    let cancelled = false;
    async function bootstrap() {
      try {
        if (!sessionId) {
          const current = await backend.getCurrentSession();
          if (current.sessionId && current.uiUrl && current.state) {
            if (cancelled) return;
            replaceUrl(current.uiUrl);
            applySessionSnapshot(current.state, { forceSource: true });
            await backend.markSessionViewed(current.sessionId);
            return;
          }
          const created = await backend.createSession({ title: "Cadastrophe review" });
          if (cancelled) return;
          replaceUrl(created.uiUrl);
          applySessionSnapshot(created.state, { forceSource: true });
          await backend.markSessionViewed(created.sessionId);
          return;
        }
        await loadSession(sessionId);
        await backend.markSessionViewed(sessionId);
      } catch (caught) {
        if (!cancelled) setError(errorMessage(caught));
      }
    }
    bootstrap();
    return () => {
      cancelled = true;
    };
  }, [applySessionSnapshot, backend, loadSession, sessionId]);

  useEffect(() => {
    if (!state?.session.id) return;
    const sessionId = state.session.id;
    const unsubscribe = backend.subscribeSession(sessionId, {
      onStatus: (status) => {
        setConnectionStatus(status);
        if (status === "connected") {
          loadSession(sessionId).catch((caught) => setError(errorMessage(caught)));
        }
      },
      onSnapshot: applySessionSnapshot,
      onError: (caught) => setError(errorMessage(caught))
    });
    return unsubscribe;
  }, [applySessionSnapshot, backend, loadSession, state?.session.id]);

  useEffect(() => {
    let cancelled = false;
    if (!previewArtifact) {
      setMesh(null);
      return;
    }
    backend.readPreviewMesh(previewArtifact).then((nextMesh) => {
      if (!cancelled) setMesh(nextMesh);
    }).catch((caught) => setError(errorMessage(caught)));
    return () => {
      cancelled = true;
    };
  }, [backend, previewArtifact?.id]);

  useEffect(() => {
    if (view !== "sessions") return;
    let cancelled = false;
    backend
      .listSessions({ includeArchived: true, query: sessionSearch })
      .then((result) => {
        if (cancelled) return;
        setSessionList(filterLocallyDeletedSessions(result.sessions));
        setSessionSearchFields(result.searchFields);
      })
      .catch((caught) => setError(errorMessage(caught)));
    return () => {
      cancelled = true;
    };
  }, [backend, sessionSearch, state?.session.updatedAt, view]);

  async function saveSource() {
    if (!state || sessionArchived) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.updateModelSource({
        sessionId: state.session.id,
        sourceLanguage: "openscad",
        source,
        parentRevisionId: state.session.activeRevisionId
      });
      applySessionSnapshot(result.state, { forceSource: true });
    });
  }

  async function renderPreview() {
    if (!state || sessionArchived) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const revision = await ensureCurrentSourceRevision();
      const rendered = await renderOpenScadInWorker(revision.source, revision.parameters, setRuntimeState);
      previewCacheRef.current = { ...rendered, revisionId: revision.id };
      setMesh(rendered.mesh);
      const persisted = await backend.persistRuntimeArtifact({
        sessionId: state.session.id,
        revisionId: revision.id,
        kind: "preview-mesh",
        format: "json",
        contentsBase64: base64EncodeUtf8(JSON.stringify(rendered.mesh)),
        diagnostics: rendered.diagnostics,
        metadata: runtimeMetadata(rendered, "preview")
      });
      applySessionSnapshot(persisted.state);
    });
  }

  async function updateParameter(parameter: CadParameter, value: CadParameter["value"]) {
    if (!state || sessionArchived) return;
    await resyncIfDisconnected();
    const result = await backend.updateParameters({
      sessionId: state.session.id,
      values: { [parameter.name]: value }
    });
    applySessionSnapshot(result);
  }

  async function startAgentRun(promptOverride?: string, retryOfRunId?: string) {
    if (!state || sessionArchived) return;
    const prompt = (promptOverride ?? agentPrompt).trim();
    if (!prompt) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      await backend.createAgentRun({
        sessionId: state.session.id,
        prompt,
        revisionId: state.session.activeRevisionId,
        retryOfRunId
      });
      if (!promptOverride) setAgentPrompt("");
      await loadSession(state.session.id);
    });
  }

  async function cancelAgentRun(runId: string) {
    if (!state || sessionArchived) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.cancelAgentRun({ sessionId: state.session.id, runId });
      applySessionSnapshot(result.state);
    });
  }

  async function exportArtifact(format: "stl" | "metadata", revisionId = state?.session.activeRevisionId) {
    if (!state || sessionArchived) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      if (format === "stl") {
        const revision = await ensureCurrentSourceRevision(revisionId);
        const rendered = await cachedOrRenderedStl(revision);
        const persisted = await backend.persistRuntimeArtifact({
          sessionId: state.session.id,
          revisionId: revision.id,
          kind: "stl",
          format,
          contentsBase64: base64EncodeBytes(rendered.stlBytes),
          diagnostics: rendered.diagnostics,
          metadata: runtimeMetadata(rendered, "export")
        });
        applySessionSnapshot(persisted.state);
        return;
      }
      const result = await backend.exportArtifact({ sessionId: state.session.id, revisionId, format });
      applySessionSnapshot(result.state);
    });
  }

  async function ensureCurrentSourceRevision(revisionId = state?.session.activeRevisionId): Promise<CadRevision> {
    if (!state) throw new Error("No active session is loaded.");
    if (sourceDirty && revisionId === state.session.activeRevisionId) {
      const result = await backend.updateModelSource({
        sessionId: state.session.id,
        sourceLanguage: "openscad",
        source,
        parentRevisionId: state.session.activeRevisionId
      });
      applySessionSnapshot(result.state, { forceSource: true });
      const revision = result.state.activeRevision;
      if (!revision) throw new Error("Saved source revision is not available.");
      return revision;
    }
    const revision =
      revisionId === state.activeRevision?.id
        ? state.activeRevision
        : state.session.revisions.find((candidate) => candidate.id === revisionId) && state.activeRevision?.id === revisionId
          ? state.activeRevision
          : undefined;
    if (!revision) throw new Error("The requested revision is not active. Activate it before rendering.");
    return revision;
  }

  async function cachedOrRenderedStl(revision: CadRevision) {
    const parameterHash = await sha256Hex(JSON.stringify(parameterValues(revision.parameters)));
    const cache = previewCacheRef.current;
    if (cache?.revisionId === revision.id && cache.sourceHash === revision.sourceHash && cache.parameterHash === parameterHash) {
      return cache;
    }
    const rendered = await renderOpenScadInWorker(revision.source, revision.parameters, setRuntimeState);
    previewCacheRef.current = { ...rendered, revisionId: revision.id };
    setMesh(rendered.mesh);
    return rendered;
  }

  async function openArtifact(artifactId: string) {
    await runBusy(async () => {
      const result = await backend.openArtifact(artifactId);
      setOpenedArtifactPath(result.path);
    });
  }

  async function deleteArtifact(artifactId: string) {
    if (!state || sessionArchived) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.deleteArtifact({
        sessionId: state.session.id,
        artifactId
      });
      setOpenedArtifactPath(null);
      applySessionSnapshot(result.state);
    });
  }

  async function verifyArtifacts() {
    if (!state) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.verifyArtifactFiles({ sessionId: state.session.id });
      setIntegrityResult(result);
      if (result.state) {
        applySessionSnapshot(result.state);
      } else {
        await loadSession(state.session.id);
      }
    });
  }

  async function setActiveRevision(revisionId: string) {
    if (!state || sessionArchived || revisionId === state.session.activeRevisionId || sourceDirty) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.setActiveRevision({
        sessionId: state.session.id,
        revisionId
      });
      applySessionSnapshot(result, { forceSource: true });
    });
  }

  async function restoreRevision(revisionId: string) {
    if (!state || sessionArchived) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.restoreRevision({
        sessionId: state.session.id,
        revisionId
      });
      applySessionSnapshot(result.state, { forceSource: true });
    });
  }

  async function refreshSessionList(query = sessionSearch) {
    const result = await backend.listSessions({ includeArchived: true, query });
    setSessionList(filterLocallyDeletedSessions(result.sessions));
    setSessionSearchFields(result.searchFields);
    return result;
  }

  async function openSession(sessionId: string) {
    await runBusy(async () => {
      const nextState = await loadSession(sessionId, { forceSource: true });
      await backend.markSessionViewed(sessionId);
      setOpenedArtifactPath(null);
      navigateTo("workspace", nextState.session.id);
    });
  }

  async function createNewSession() {
    await runBusy(async () => {
      const created = await backend.createSession({ title: "Cadastrophe review" });
      applySessionSnapshot(created.state, { forceSource: true });
      await backend.markSessionViewed(created.sessionId);
      setOpenedArtifactPath(null);
      navigateTo("workspace", created.sessionId);
    });
  }

  async function setSessionArchived(sessionId: string, archived: boolean) {
    await runBusy(async () => {
      const nextState = await backend.archiveSession({ sessionId, archived });
      if (state?.session.id === sessionId) {
        applySessionSnapshot(nextState, { forceSource: true });
      }
      await refreshSessionList();
    });
  }

  async function renameSession(sessionId: string, title: string) {
    await runBusy(async () => {
      const nextState = await backend.renameSession({ sessionId, title });
      if (state?.session.id === sessionId) {
        applySessionSnapshot(nextState, { forceSource: true });
      }
      await refreshSessionList();
    });
  }

  async function duplicateSession(sessionId: string) {
    await runBusy(async () => {
      const duplicated = await backend.duplicateSession({ sessionId });
      applySessionSnapshot(duplicated.state, { forceSource: true });
      await backend.markSessionViewed(duplicated.sessionId);
      setOpenedArtifactPath(null);
      navigateTo("workspace", duplicated.sessionId);
    });
  }

  async function deleteSession(sessionId: string) {
    if (!window.confirm("Delete this session and its local management data?")) return;
    locallyDeletedSessionIdsRef.current.add(sessionId);
    setSessionList((sessions) => sessions.filter((session) => session.id !== sessionId));
    await runBusy(async () => {
      const deleted = await backend.deleteSession(sessionId).catch(async (caught) => {
        locallyDeletedSessionIdsRef.current.delete(sessionId);
        await refreshSessionList().catch(() => undefined);
        throw caught;
      });
      setOpenedArtifactPath(null);
      await refreshSessionList();
      if (state?.session.id !== sessionId) {
        return;
      }
      const nextSessionId =
        deleted.currentSessionId ??
        (await backend.listSessions({ includeArchived: false })).sessions[0]?.id;
      if (nextSessionId) {
        const nextState = await loadSession(nextSessionId, { forceSource: true });
        await backend.markSessionViewed(nextSessionId);
        navigateTo(view, nextState.session.id);
        return;
      }
      const created = await backend.createSession({ title: "Cadastrophe review" });
      applySessionSnapshot(created.state, { forceSource: true });
      await backend.markSessionViewed(created.sessionId);
      await refreshSessionList();
      navigateTo(view, created.sessionId);
    });
  }

  function filterLocallyDeletedSessions(sessions: CadSessionListItem[]) {
    const deletedSessionIds = locallyDeletedSessionIdsRef.current;
    if (deletedSessionIds.size === 0) return sessions;
    return sessions.filter((session) => !deletedSessionIds.has(session.id));
  }

  function navigateTo(nextView: WorkspaceView, targetSessionId = state?.session.id) {
    setView(nextView);
    if (targetSessionId) {
      window.history.replaceState({}, "", sessionPathWithView(targetSessionId, nextView));
    }
  }

  async function resyncIfDisconnected() {
    if (state?.session.id && connectionStatus !== "connected") {
      await loadSession(state.session.id);
    }
  }

  function editSource(value: string) {
    setSource(value);
    setSourceDirty(true);
    sourceDirtyRef.current = true;
  }

  function useLatestSource() {
    if (!state) return;
    applySessionSnapshot(state, { forceSource: true });
  }

  function keepLocalSource() {
    setSourceConflict(false);
  }

  async function runBusy(work: () => Promise<void>) {
    setBusy(true);
    setError(null);
    try {
      await work();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  }

  if (!state) {
    return (
      <main className="loading">
        <div>
          <p>Loading Cadastrophe session</p>
          {error ? <div className="error">{error}</div> : null}
        </div>
      </main>
    );
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div>
          <h1>{state.session.title}</h1>
          <p>{state.session.id}</p>
        </div>
        <div className="status-group">
          <div className={`status status-${state.session.status}`} data-testid="session-status">
            {state.session.status.replaceAll("_", " ")}
          </div>
          <div className={`connection connection-${connectionStatus}`}>{connectionStatus}</div>
        </div>
      </header>

      {error ? <div className="error">{error}</div> : null}
      {sourceConflict ? (
        <div className="warning" role="alert">
          <span>The agent created a newer revision while this source has unsaved edits.</span>
          <button onClick={useLatestSource} title="Use server revision">
            Use latest
          </button>
          <button onClick={keepLocalSource} title="Keep local edits">
            Keep edits
          </button>
        </div>
      ) : null}

      <WorkspaceNav
        busy={busy}
        onCreateSession={createNewSession}
        onNavigate={navigateTo}
        view={view}
      />

      {sessionArchived ? (
        <div className="warning" role="status">
          <span>Archived session. Open Sessions to unarchive before editing.</span>
          <button onClick={() => navigateTo("sessions")} title="Open sessions">
            <List size={16} /> Sessions
          </button>
        </div>
      ) : null}

      {view === "workspace" ? (
        <WorkspacePanel
          state={state}
          mesh={mesh}
          runtimeState={runtimeState}
          busy={busy}
          sessionArchived={sessionArchived}
          source={source}
          sourceDirty={sourceDirty}
          activeRevision={activeRevision}
          activeRun={activeAgentRun}
          agentPrompt={agentPrompt}
          onRenderPreview={renderPreview}
          onSaveSource={saveSource}
          onEditSource={editSource}
          onPromptChange={setAgentPrompt}
          onStartRun={() => startAgentRun()}
          onRetryRun={(run) => startAgentRun(run.prompt, run.id)}
          onCancelRun={cancelAgentRun}
          onUpdateParameter={updateParameter}
          onActivateRevision={setActiveRevision}
          onRestoreRevision={restoreRevision}
        />
      ) : null}

      {view === "sessions" ? (
        <SessionBrowser
          sessions={sessionList}
          activeSessionId={state.session.id}
          query={sessionSearch}
          searchFields={sessionSearchFields}
          busy={busy}
          onQueryChange={setSessionSearch}
          onOpen={openSession}
          onArchiveChange={setSessionArchived}
          onRename={renameSession}
          onDuplicate={duplicateSession}
          onDelete={deleteSession}
        />
      ) : null}

      {view === "artifacts" ? (
        <ArtifactBrowser
          revisions={state.session.revisions}
          activeRevisionId={state.session.activeRevisionId}
          artifacts={activeRevision?.artifacts ?? []}
          busy={busy}
          readOnly={sessionArchived}
          sourceDirty={sourceDirty}
          openedPath={openedArtifactPath}
          integrityResult={integrityResult}
          onExport={exportArtifact}
          onVerify={verifyArtifacts}
          onActivateRevision={setActiveRevision}
          onOpen={openArtifact}
          onDelete={deleteArtifact}
        />
      ) : null}

      {view === "logs" ? (
        <SessionLogs
          conversation={state.conversation}
          runs={state.agentRuns}
          events={state.agentRunEvents}
          workflow={state.workflow}
        />
      ) : null}
    </main>
  );
}
