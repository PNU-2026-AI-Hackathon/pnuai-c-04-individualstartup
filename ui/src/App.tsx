import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Archive,
  ArchiveRestore,
  Box,
  Check,
  Clock,
  Copy,
  GitCompare,
  Download,
  Edit3,
  FolderOpen,
  Home,
  List,
  Plus,
  Play,
  RefreshCcw,
  RotateCcw,
  Save,
  Search,
  Send,
  ScrollText,
  SquareMousePointer,
  Trash2,
  X
} from "lucide-react";
import type {
  CadAgentRun,
  CadAgentRunEvent,
  CadArtifact,
  CadConversationMessage,
  CadMesh,
  CadParameter,
  CadRevision,
  CadRevisionSummary,
  CadSessionListItem,
  CadSessionState,
  CadWorkflowOuterIteration,
  CadWorkflowPendingVlm,
  CadWorkflowState,
  VerifyArtifactFilesResult
} from "./protocol";
import { createCadBackendClient, type ConnectionStatus } from "./backendClient";
import { MeshPreview } from "./MeshPreview";
import {
  sessionIdFromUrl,
  sessionPathWithView,
  toHistoryPath,
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
        <section className="workspace">
          <div className="preview-pane">
            <div className="pane-toolbar">
              <h2>Preview</h2>
              <span className={`runtime-state runtime-state-${runtimeState}`}>{runtimeState}</span>
              <button onClick={renderPreview} disabled={busy || sessionArchived} title="Render preview">
                <Play size={16} /> Render
              </button>
            </div>
            <MeshPreview mesh={mesh} />
            <Diagnostics state={state} />
          </div>

          <div className="editor-pane">
            <div className="pane-toolbar">
              <h2>OpenSCAD Source</h2>
              <button onClick={saveSource} disabled={busy || !sourceDirty || sessionArchived} title="Save source revision">
                <Save size={16} /> Save
              </button>
            </div>
            <textarea
              data-testid="source-editor"
              value={source}
              onChange={(event) => editSource(event.target.value)}
              readOnly={sessionArchived}
              spellCheck={false}
            />
          </div>

          <aside className="side-pane">
            <AgentWorkspace
              conversation={state.conversation}
              runs={state.agentRuns}
              events={state.agentRunEvents}
              workflow={state.workflow}
              prompt={agentPrompt}
              busy={busy}
              readOnly={sessionArchived}
              activeRun={activeAgentRun}
              onPromptChange={setAgentPrompt}
              onStartRun={() => startAgentRun()}
              onRetryRun={(run) => startAgentRun(run.prompt, run.id)}
              onCancelRun={cancelAgentRun}
            />
            <Parameters revision={activeRevision} readOnly={sessionArchived} onUpdate={updateParameter} />
            <Timeline
              state={state}
              busy={busy}
              readOnly={sessionArchived}
              sourceDirty={sourceDirty}
              onActivate={setActiveRevision}
              onRestore={restoreRevision}
            />
          </aside>
        </section>
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

function WorkspaceNav({
  busy,
  onCreateSession,
  view,
  onNavigate
}: {
  busy: boolean;
  onCreateSession: () => void;
  view: WorkspaceView;
  onNavigate: (view: WorkspaceView) => void;
}) {
  const items: Array<{ view: WorkspaceView; label: string; icon: typeof Home }> = [
    { view: "workspace", label: "Workspace", icon: Home },
    { view: "sessions", label: "Sessions", icon: List },
    { view: "artifacts", label: "Artifacts", icon: Box },
    { view: "logs", label: "Logs", icon: ScrollText }
  ];
  return (
    <nav className="workspace-nav" aria-label="Workspace navigation">
      {items.map((item) => {
        const Icon = item.icon;
        return (
          <button
            className={view === item.view ? "active" : ""}
            key={item.view}
            onClick={() => onNavigate(item.view)}
            title={item.label}
          >
            <Icon size={16} /> {item.label}
          </button>
        );
      })}
      <button
        className="workspace-nav-action"
        onClick={onCreateSession}
        disabled={busy}
        title="Create session"
      >
        <Plus size={16} /> New session
      </button>
    </nav>
  );
}

function SessionBrowser({
  sessions,
  activeSessionId,
  query,
  searchFields,
  busy,
  onQueryChange,
  onOpen,
  onArchiveChange,
  onRename,
  onDuplicate,
  onDelete
}: {
  sessions: CadSessionListItem[];
  activeSessionId: string;
  query: string;
  searchFields: string[];
  busy: boolean;
  onQueryChange: (query: string) => void;
  onOpen: (sessionId: string) => void;
  onArchiveChange: (sessionId: string, archived: boolean) => void;
  onRename: (sessionId: string, title: string) => void;
  onDuplicate: (sessionId: string) => void;
  onDelete: (sessionId: string) => void;
}) {
  const [editingSessionId, setEditingSessionId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");

  function startRename(session: CadSessionListItem) {
    setEditingSessionId(session.id);
    setDraftTitle(session.title ?? "Untitled CAD session");
  }

  function submitRename(sessionId: string) {
    const title = draftTitle.trim();
    if (!title) return;
    onRename(sessionId, title);
    setEditingSessionId(null);
  }

  return (
    <section className="management-view" data-testid="session-browser">
      <div className="management-toolbar">
        <label className="search-field">
          <Search size={16} />
          <input
            aria-label={`Search sessions by ${searchFields.join(", ") || "title and source"}`}
            value={query}
            onChange={(event) => onQueryChange(event.target.value)}
            placeholder="Search sessions"
          />
        </label>
      </div>
      <ol className="session-list">
        {sessions.map((session) => {
          const isEditing = editingSessionId === session.id;
          return (
            <li className={session.id === activeSessionId ? "active" : ""} key={session.id}>
              <div className="session-list-main">
                {isEditing ? (
                  <div className="rename-row">
                    <input
                      aria-label={`Rename session ${session.id}`}
                      value={draftTitle}
                      onChange={(event) => setDraftTitle(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") submitRename(session.id);
                        if (event.key === "Escape") setEditingSessionId(null);
                      }}
                    />
                    <button
                      aria-label={`Save session title ${session.id}`}
                      disabled={busy || !draftTitle.trim()}
                      onClick={() => submitRename(session.id)}
                      title="Save session title"
                    >
                      <Check size={16} />
                    </button>
                    <button
                      aria-label={`Cancel rename ${session.id}`}
                      onClick={() => setEditingSessionId(null)}
                      title="Cancel rename"
                    >
                      <X size={16} />
                    </button>
                  </div>
                ) : (
                  <button
                    className="session-title-button"
                    onClick={() => onOpen(session.id)}
                    disabled={busy}
                    title="Open session"
                  >
                    {session.title ?? "Untitled CAD session"}
                  </button>
                )}
                <span>{session.id}</span>
                <div className="session-list-meta">
                  <small><Clock size={13} /> {new Date(session.updatedAt).toLocaleString()}</small>
                  <small>{session.revisionCount} revisions</small>
                  <small>{session.artifactCount} artifacts</small>
                  {session.archived ? <small><Archive size={13} /> archived</small> : null}
                </div>
                {session.activeRevision ? (
                  <div className="active-revision-summary">
                    <code>{session.activeRevision.id.slice(0, 8)}</code>
                    <span>{session.activeRevision.sourceLanguage}</span>
                    <span>{session.activeRevision.artifactCount} artifacts</span>
                    <span>{session.activeRevision.sourceHash.slice(0, 10)}</span>
                  </div>
                ) : null}
              </div>
              <div className="session-list-actions">
                <button onClick={() => startRename(session)} disabled={busy} title="Rename session">
                  <Edit3 size={16} /> Rename
                </button>
                <button onClick={() => onDuplicate(session.id)} disabled={busy} title="Duplicate session">
                  <Copy size={16} /> Duplicate
                </button>
                <button
                  onClick={() => onArchiveChange(session.id, !session.archived)}
                  disabled={busy}
                  title={session.archived ? "Unarchive session" : "Archive session"}
                >
                  {session.archived ? <ArchiveRestore size={16} /> : <Archive size={16} />}
                  {session.archived ? "Unarchive" : "Archive"}
                </button>
                <button onClick={() => onDelete(session.id)} disabled={busy} title="Delete session">
                  <Trash2 size={16} /> Delete
                </button>
              </div>
            </li>
          );
        })}
      </ol>
      {sessions.length === 0 ? <p className="empty-state">No sessions found.</p> : null}
    </section>
  );
}

function ArtifactBrowser({
  revisions,
  activeRevisionId,
  artifacts,
  busy,
  readOnly,
  sourceDirty,
  openedPath,
  integrityResult,
  onExport,
  onVerify,
  onActivateRevision,
  onOpen,
  onDelete
}: {
  revisions: CadRevisionSummary[];
  activeRevisionId?: string;
  artifacts: CadArtifact[];
  busy: boolean;
  readOnly: boolean;
  sourceDirty: boolean;
  openedPath: string | null;
  integrityResult: VerifyArtifactFilesResult | null;
  onExport: (format: "stl" | "metadata", revisionId?: string) => void;
  onVerify: () => void;
  onActivateRevision: (revisionId: string) => void;
  onOpen: (artifactId: string) => void;
  onDelete: (artifactId: string) => void;
}) {
  return (
    <section className="management-view artifact-browser" data-testid="artifact-browser">
      <div className="management-toolbar">
        <h2>Artifacts</h2>
        <div className="button-row compact">
          <button onClick={() => onExport("stl")} disabled={busy || readOnly} title="Export STL">
            <Download size={16} /> STL
          </button>
          <button onClick={() => onExport("metadata")} disabled={busy || readOnly} title="Export metadata">
            <RefreshCcw size={16} /> Metadata
          </button>
          <button onClick={onVerify} disabled={busy} title="Check artifact integrity">
            <Search size={16} /> Check
          </button>
        </div>
      </div>
      {integrityResult ? <IntegrityResult result={integrityResult} /> : null}
      <RevisionArtifactScope
        revisions={revisions}
        activeRevisionId={activeRevisionId}
        busy={busy}
        readOnly={readOnly}
        sourceDirty={sourceDirty}
        onActivate={onActivateRevision}
      />
      <ArtifactList
        artifacts={artifacts}
        busy={busy}
        readOnly={readOnly}
        openedPath={openedPath}
        integrityResult={integrityResult}
        onExport={onExport}
        onOpen={onOpen}
        onDelete={onDelete}
      />
    </section>
  );
}

function RevisionArtifactScope({
  revisions,
  activeRevisionId,
  busy,
  readOnly,
  sourceDirty,
  onActivate
}: {
  revisions: CadRevisionSummary[];
  activeRevisionId?: string;
  busy: boolean;
  readOnly: boolean;
  sourceDirty: boolean;
  onActivate: (revisionId: string) => void;
}) {
  return (
    <div className="artifact-revision-scope" aria-label="Artifact revision scope">
      {revisions.map((revision) => (
        <button
          className={revision.id === activeRevisionId ? "active" : ""}
          disabled={busy || readOnly || sourceDirty || revision.id === activeRevisionId}
          key={revision.id}
          onClick={() => onActivate(revision.id)}
          title={sourceDirty ? "Save or discard source edits before changing revisions" : "Show artifacts for this revision"}
        >
          <span>{revision.id.slice(0, 8)}</span>
          <small>{revision.artifactCount}</small>
        </button>
      ))}
    </div>
  );
}

function IntegrityResult({ result }: { result: VerifyArtifactFilesResult }) {
  const issueCount =
    result.missingArtifactIds.length +
    result.hashMismatchArtifactIds.length +
    result.sizeMismatchArtifactIds.length +
    result.corruptMetadataArtifactIds.length +
    result.invalidPathArtifactIds.length +
    result.orphanPaths.length;
  return (
    <section className={issueCount ? "integrity-result integrity-warning" : "integrity-result"} data-testid="integrity-result">
      <strong>{issueCount ? `${issueCount} integrity issues` : "Integrity check passed"}</strong>
      <span>{result.checkedCount} manifests checked</span>
      {result.diagnostics.length ? (
        <ol>
          {result.diagnostics.slice(0, 6).map((diagnostic, index) => (
            <li className={`diagnostic-${diagnostic.severity}`} key={`${diagnostic.message}-${index}`}>
              {diagnostic.severity}: {diagnostic.message}
            </li>
          ))}
        </ol>
      ) : null}
    </section>
  );
}

function SessionLogs({
  conversation,
  runs,
  events,
  workflow
}: {
  conversation: CadConversationMessage[];
  runs: CadAgentRun[];
  events: CadAgentRunEvent[];
  workflow: CadWorkflowState;
}) {
  return (
    <section className="management-view log-browser" data-testid="log-browser">
      <div className="conversation-log">
        <h2>Conversation</h2>
        <ol className="conversation expanded">
          {conversation.map((message) => (
            <li className={`conversation-item conversation-${message.role}`} key={message.id}>
              <span>{message.role}</span>
              <p>{message.content}</p>
              <small>{new Date(message.createdAt).toLocaleString()}</small>
            </li>
          ))}
        </ol>
      </div>
      <RunLogViewer runs={runs} events={events} conversation={conversation} workflow={workflow} />
    </section>
  );
}

function AgentWorkspace(props: {
  conversation: CadConversationMessage[];
  runs: CadAgentRun[];
  events: CadAgentRunEvent[];
  workflow: CadWorkflowState;
  prompt: string;
  busy: boolean;
  readOnly: boolean;
  activeRun?: CadAgentRun;
  onPromptChange: (value: string) => void;
  onStartRun: () => void;
  onRetryRun: (run: CadAgentRun) => void;
  onCancelRun: (runId: string) => void;
}) {
  const latestRun = props.runs.at(-1);
  const conversation = buildAgentConversation(props.conversation);
  const promptDisabled = props.busy || props.readOnly || Boolean(props.activeRun);
  const latestRunEvents = latestRun
    ? props.events
        .filter((event) => event.runId === latestRun.id)
        .sort((left, right) => left.sequence - right.sequence)
    : [];
  const latestWorkflow = latestRun
    ? workflowRunView(latestRun, latestRunEvents, props.workflow)
    : undefined;
  return (
    <section className="panel agent-workspace">
      <div className="panel-heading">
        <h2>Codex Agent</h2>
      </div>
      {props.activeRun?.activeStep ? (
        <div className="active-step" data-testid="active-step">
          {props.activeRun.activeStep.replaceAll("_", " ")}
        </div>
      ) : null}
      {latestRun && latestWorkflow ? (
        <WorkflowRunSummary run={latestRun} view={latestWorkflow} compact />
      ) : null}
      {latestRun && latestWorkflow ? (
        <AgentRunProgressDetails run={latestRun} events={latestRunEvents} view={latestWorkflow} />
      ) : null}
      <ol className="conversation" data-testid="conversation-timeline">
        {conversation.map((message) => (
          <li className={`conversation-item conversation-${message.role}`} key={message.id}>
            <span>{message.role}</span>
            <p>{message.content}</p>
            <small>{new Date(message.createdAt).toLocaleTimeString()}</small>
          </li>
        ))}
      </ol>
      <textarea
        className="small-textarea"
        aria-label="Ask Codex agent"
        data-testid="agent-prompt"
        value={props.prompt}
        onChange={(event) => props.onPromptChange(event.target.value)}
        readOnly={props.readOnly}
        placeholder={props.activeRun ? "Agent run in progress" : "Ask Codex to create or revise the CAD model"}
      />
      <div className="button-row">
        <button
          data-testid="send-agent-prompt"
          onClick={props.onStartRun}
          disabled={promptDisabled || !props.prompt.trim()}
          title="Start agent run"
        >
          <Send size={16} /> Run
        </button>
        <button
          data-testid="cancel-agent-run"
          onClick={() => props.activeRun && props.onCancelRun(props.activeRun.id)}
          disabled={props.busy || props.readOnly || !props.activeRun}
          title="Cancel agent run"
        >
          <X size={16} /> Cancel
        </button>
      </div>
      {latestRun?.status === "failed" || latestRun?.status === "cancelled" ? (
        <button
          data-testid="retry-agent-run"
          onClick={() => props.onRetryRun(latestRun)}
          disabled={props.busy || props.readOnly || Boolean(props.activeRun)}
          title="Retry agent run"
        >
          <RefreshCcw size={16} /> Retry
        </button>
      ) : null}
    </section>
  );
}

function AgentRunProgressDetails({
  run,
  events,
  view
}: {
  run: CadAgentRun;
  events: CadAgentRunEvent[];
  view: WorkflowRunView;
}) {
  const recentEvents = events.slice(-10).reverse();
  const activeLabel = run.activeStep ?? view.latestCommand ?? view.stage;
  return (
    <details
      className="agent-progress-details"
      data-testid="agent-progress-details"
      open={isActiveRunStatus(run.status) || run.status === "failed"}
    >
      <summary>
        <span>Progress details</span>
        <small>{activeLabel.replaceAll("_", " ")}</small>
      </summary>
      <div className="agent-progress-meta">
        <div>
          <dt>status</dt>
          <dd>{run.status.replaceAll("_", " ")}</dd>
        </div>
        <div>
          <dt>stage</dt>
          <dd>{view.stage}</dd>
        </div>
        <div>
          <dt>events</dt>
          <dd>{events.length}</dd>
        </div>
      </div>
      {run.error ? <p className="agent-progress-error">{run.error}</p> : null}
      {recentEvents.length ? (
        <ol>
          {recentEvents.map((event) => (
            <li className={`event-${event.type.replaceAll(".", "-")}`} key={event.id}>
              <span>{event.sequence}. {event.type}</span>
              <small>{new Date(event.createdAt).toLocaleTimeString()}</small>
              <EventPayloadSummary event={event} />
            </li>
          ))}
        </ol>
      ) : (
        <p className="agent-progress-empty">No progress events yet.</p>
      )}
    </details>
  );
}

function buildAgentConversation(conversation: CadConversationMessage[]) {
  return conversation
    .filter((message) => message.role === "user" || message.role === "assistant")
    .sort((left, right) => left.createdAt.localeCompare(right.createdAt));
}

function isActiveRunStatus(status: CadAgentRun["status"]): boolean {
  return status === "queued" || status === "running" || status === "waiting_for_user";
}

function RunLogViewer({
  runs,
  events,
  conversation,
  workflow
}: {
  runs: CadAgentRun[];
  events: CadAgentRunEvent[];
  conversation: CadConversationMessage[];
  workflow: CadWorkflowState;
}) {
  const eventsByRun = new Map<string, CadAgentRunEvent[]>();
  for (const event of events) {
    const runEvents = eventsByRun.get(event.runId) ?? [];
    runEvents.push(event);
    eventsByRun.set(event.runId, runEvents);
  }
  const messagesByRun = new Map<string, CadConversationMessage[]>();
  for (const message of conversation) {
    if (!message.runId) continue;
    const runMessages = messagesByRun.get(message.runId) ?? [];
    runMessages.push(message);
    messagesByRun.set(message.runId, runMessages);
  }
  const groupedRuns = [...runs].reverse();
  return (
    <section className="run-log" data-testid="run-log-viewer">
      <h3>Run Log</h3>
      {groupedRuns.length ? groupedRuns.map((run) => {
        const runEvents = [...(eventsByRun.get(run.id) ?? [])].sort((left, right) => left.sequence - right.sequence);
        const runMessages = [...(messagesByRun.get(run.id) ?? [])].sort((left, right) => left.createdAt.localeCompare(right.createdAt));
        const failureEvents = runEvents.filter((event) => event.type === "agent.run.failed" || event.type === "agent.run.cancelled");
        const retryEvents = runEvents.filter((event) => event.payload.retryOfRunId);
        const workflowView = workflowRunView(run, runEvents, workflow);
        return (
          <details key={run.id} open={isActiveRunStatus(run.status) || run.status === "failed"}>
            <summary>
              <span>{run.id.slice(0, 8)}</span>
              <small>{run.status.replaceAll("_", " ")}</small>
            </summary>
            <dl className="run-log-meta">
              <div>
                <dt>input</dt>
                <dd>{shortId(run.inputRevisionId)}</dd>
              </div>
              <div>
                <dt>output</dt>
                <dd>{shortId(run.outputRevisionId)}</dd>
              </div>
              <div>
                <dt>agent</dt>
                <dd>{run.externalAgent ?? "unknown"}</dd>
              </div>
              <div>
                <dt>created</dt>
                <dd>{new Date(run.createdAt).toLocaleString()}</dd>
              </div>
              <div>
                <dt>updated</dt>
                <dd>{new Date(run.updatedAt).toLocaleString()}</dd>
              </div>
              {run.completedAt ? (
                <div>
                  <dt>completed</dt>
                  <dd>{new Date(run.completedAt).toLocaleString()}</dd>
                </div>
              ) : null}
              {run.activeStep ? (
                <div>
                  <dt>active step</dt>
                  <dd>{run.activeStep.replaceAll("_", " ")}</dd>
                </div>
              ) : null}
              {run.externalThreadId ? (
                <div>
                  <dt>thread</dt>
                  <dd>{shortId(run.externalThreadId)}</dd>
                </div>
              ) : null}
              {run.error ? (
                <div>
                  <dt>error</dt>
                  <dd>{run.error}</dd>
                </div>
              ) : null}
            </dl>
            <WorkflowRunSummary run={run} view={workflowView} />
            <div className="run-diagnostics">
              <strong>{failureEvents.length ? "Failure diagnostics" : "Run diagnostics"}</strong>
              <span>{runEvents.length} events, {runMessages.length} messages</span>
              {retryEvents.length ? <span>{retryEvents.length} retry references recorded</span> : null}
              {failureEvents.map((event) => (
                <code key={event.id}>{formatPayload(event.payload)}</code>
              ))}
            </div>
            {runMessages.length ? (
              <ol className="run-messages">
                {runMessages.map((message) => (
                  <li key={message.id}>
                    <span>{message.role}</span>
                    <p>{message.content}</p>
                    <small>{new Date(message.createdAt).toLocaleTimeString()}</small>
                  </li>
                ))}
              </ol>
            ) : null}
            <ol>
              {runEvents.map((event) => (
                <li className={`event-${event.type.replaceAll(".", "-")}`} key={event.id}>
                  <span>{event.sequence}. {event.type}</span>
                  <small>{new Date(event.createdAt).toLocaleTimeString()}</small>
                  <EventPayloadSummary event={event} />
                  {Object.keys(event.payload).length ? <code>{formatPayload(event.payload)}</code> : null}
                  {event.metadata && Object.keys(event.metadata).length ? <code>{formatPayload(event.metadata)}</code> : null}
                </li>
              ))}
            </ol>
          </details>
        );
      }) : <p>No runs yet.</p>}
    </section>
  );
}

interface WorkflowRunView {
  stage: string;
  finalizationStatus: string;
  plan?: CadWorkflowState["plans"][number];
  iterations: CadWorkflowOuterIteration[];
  pendingVlm?: CadWorkflowPendingVlm;
  latestFailure?: Record<string, unknown>;
  latestCommand?: string;
  latestNextAction?: string;
}

function WorkflowRunSummary({
  run,
  view,
  compact = false
}: {
  run: CadAgentRun;
  view: WorkflowRunView;
  compact?: boolean;
}) {
  return (
    <div className={compact ? "workflow-summary compact" : "workflow-summary"} data-testid="workflow-summary">
      <div className="workflow-summary-grid">
        <div>
          <dt>workflow</dt>
          <dd>{view.stage}</dd>
        </div>
        <div>
          <dt>finalization</dt>
          <dd>{view.finalizationStatus}</dd>
        </div>
        <div>
          <dt>plan</dt>
          <dd>{view.plan ? view.plan.plan.mainComponent.name : "not committed"}</dd>
        </div>
        <div>
          <dt>outer loop</dt>
          <dd>{view.iterations.length ? `${view.iterations.length} iteration${view.iterations.length === 1 ? "" : "s"}` : "none"}</dd>
        </div>
      </div>
      {view.pendingVlm ? (
        <div className="workflow-callout workflow-pending">
          <strong>Pending VLM</strong>
          <span>artifact {shortId(view.pendingVlm.artifactId)} - threshold {view.pendingVlm.passThreshold}</span>
          {contractType(view.pendingVlm.contract) ? <code>{contractType(view.pendingVlm.contract)}</code> : null}
        </div>
      ) : null}
      {view.latestFailure ? (
        <div className="workflow-callout workflow-failure">
          <strong>{failureTitle(view.latestFailure)}</strong>
          <span>{failureSummary(view.latestFailure)}</span>
          {!compact ? <code>{formatPayload(view.latestFailure)}</code> : null}
        </div>
      ) : null}
      {view.latestCommand || view.latestNextAction ? (
        <div className="workflow-last-command">
          {view.latestCommand ? <span>{view.latestCommand}</span> : null}
          {view.latestNextAction ? <small>next {view.latestNextAction.replaceAll("_", " ")}</small> : null}
        </div>
      ) : run.status === "queued" ? (
        <div className="workflow-last-command">
          <span>waiting for plan commit</span>
        </div>
      ) : null}
    </div>
  );
}

function EventPayloadSummary({ event }: { event: CadAgentRunEvent }) {
  const command = stringField(event.payload, "command") ?? stringField(event.payload, "tool");
  const status = stringField(event.payload, "status");
  const progress = stringField(event.payload, "progressLabel");
  const message = stringField(event.payload, "message");
  const nextAction = stringField(event.payload, "nextAction") ?? stringField(event.payload, "next_action");
  const contract = stringField(event.payload, "contractType") ?? stringField(event.payload, "contract_type");
  const failure = recordField(event.payload, "failureReport") ?? recordField(event.payload, "failure_report");
  const diagnostics = recordField(event.payload, "diagnostics");
  const error = recordField(event.payload, "error");
  if (!command && !status && !progress && !message && !nextAction && !contract && !failure && !diagnostics && !error) {
    return null;
  }
  return (
    <div className="event-readable">
      {command ? <strong>{command}</strong> : null}
      {progress && !command ? <strong>{progress}</strong> : null}
      {status ? <span>{status}</span> : null}
      {message ? <span>{message}</span> : null}
      {nextAction ? <span>next {nextAction.replaceAll("_", " ")}</span> : null}
      {contract ? <span>{contract}</span> : null}
      {diagnostics ? <span>{diagnosticsSummary(diagnostics)}</span> : null}
      {failure ? <span>{failureSummary(failure)}</span> : null}
      {error ? <span>{failureSummary(error)}</span> : null}
    </div>
  );
}

function workflowRunView(
  run: CadAgentRun,
  events: CadAgentRunEvent[],
  workflow: CadWorkflowState
): WorkflowRunView {
  const plan = workflow.plans.find((item) => item.runId === run.id);
  const iterations = workflow.outerIterations
    .filter((item) => item.runId === run.id)
    .sort((left, right) => left.iteration - right.iteration);
  const pendingVlm = workflow.pendingVlm.find((item) => item.runId === run.id);
  const latestFailure = [...iterations].reverse().find((iteration) => iteration.failureReport)?.failureReport;
  const latestCompletedCommand = [...events]
    .reverse()
    .find((event) => event.type === "agent.tool.completed" && (event.payload.command || event.payload.tool));
  const latestCommand = latestCompletedCommand
    ? stringField(latestCompletedCommand.payload, "command") ?? stringField(latestCompletedCommand.payload, "tool")
    : undefined;
  const latestNextAction = latestCompletedCommand
    ? stringField(latestCompletedCommand.payload, "nextAction") ?? stringField(latestCompletedCommand.payload, "next_action")
    : undefined;
  return {
    stage: workflowStage(run, events, Boolean(plan), iterations, pendingVlm, latestFailure),
    finalizationStatus: finalizationStatus(run, events, iterations, pendingVlm, latestFailure),
    plan,
    iterations,
    pendingVlm,
    latestFailure,
    latestCommand,
    latestNextAction
  };
}

function workflowStage(
  run: CadAgentRun,
  events: CadAgentRunEvent[],
  hasPlan: boolean,
  iterations: CadWorkflowOuterIteration[],
  pendingVlm?: CadWorkflowPendingVlm,
  latestFailure?: Record<string, unknown>
): string {
  if (pendingVlm) return "VLM pending";
  if (iterations.some((iteration) => iteration.passed)) return "VLM accepted";
  if (latestFailure) return failureReason(latestFailure).includes("vlm") ? "VLM repair" : "Structural repair";
  if (hasCompletedCommand(events, "cadastrophe-finalize")) return "Finalized";
  if (hasCompletedCommand(events, "cadastrophe-preview-render")) return "Preview rendered";
  if (hasCompletedCommand(events, "cadastrophe-source-apply")) return "Source applied";
  if (hasPlan) return "Plan committed";
  if (isActiveRunStatus(run.status)) return "Planning";
  return "Plan required";
}

function finalizationStatus(
  run: CadAgentRun,
  events: CadAgentRunEvent[],
  iterations: CadWorkflowOuterIteration[],
  pendingVlm?: CadWorkflowPendingVlm,
  latestFailure?: Record<string, unknown>
): string {
  if (iterations.some((iteration) => iteration.passed)) return "passed";
  if (pendingVlm) return "waiting for VLM";
  if (latestFailure) return "failed";
  if (hasCompletedCommand(events, "cadastrophe-finalize")) return "structural passed";
  if (run.status === "completed") return "completed";
  if (run.status === "failed" || run.status === "cancelled") return run.status;
  return "not finalized";
}

function hasCompletedCommand(events: CadAgentRunEvent[], command: string): boolean {
  return events.some((event) => {
    if (event.type !== "agent.tool.completed") return false;
    const payloadCommand = stringField(event.payload, "command") ?? stringField(event.payload, "tool");
    return payloadCommand?.startsWith(command) ?? false;
  });
}

function failureTitle(report: Record<string, unknown>): string {
  const reason = failureReason(report);
  return reason.includes("vlm") ? "VLM failure report" : "Structural failure report";
}

function failureReason(report: Record<string, unknown>): string {
  return stringField(report, "reason") ?? stringField(report, "code") ?? "";
}

function failureSummary(report: Record<string, unknown>): string {
  return stringField(report, "summary")
    ?? stringField(report, "message")
    ?? stringField(report, "reason")
    ?? contractType(report)
    ?? "report recorded";
}

function diagnosticsSummary(value: Record<string, unknown>): string {
  const ok = value.ok;
  const items = Array.isArray(value.items) ? value.items.length : undefined;
  if (typeof ok === "boolean" && typeof items === "number") {
    return `diagnostics ${ok ? "ok" : "failed"} (${items})`;
  }
  if (typeof ok === "boolean") return `diagnostics ${ok ? "ok" : "failed"}`;
  return "diagnostics recorded";
}

function contractType(value: Record<string, unknown>): string | undefined {
  return stringField(value, "contractType") ?? stringField(value, "contract_type");
}

function stringField(value: Record<string, unknown>, key: string): string | undefined {
  const field = value[key];
  return typeof field === "string" && field.trim() ? field : undefined;
}

function recordField(value: Record<string, unknown>, key: string): Record<string, unknown> | undefined {
  const field = value[key];
  return field && typeof field === "object" && !Array.isArray(field)
    ? field as Record<string, unknown>
    : undefined;
}

function shortId(value?: string): string {
  return value ? value.slice(0, 8) : "-";
}

function formatPayload(payload: Record<string, unknown>): string {
  return JSON.stringify(payload, null, 2);
}

function Parameters(props: {
  revision: CadSessionState["activeRevision"];
  readOnly: boolean;
  onUpdate: (parameter: CadParameter, value: CadParameter["value"]) => void;
}) {
  return (
    <section className="panel">
      <h2>Parameters</h2>
      {(props.revision?.parameters ?? []).map((parameter) => (
        <label className="parameter" key={parameter.name}>
          <span>{parameter.label ?? parameter.name}</span>
          {parameter.type === "number" ? (
            <input
              aria-label={parameter.label ?? parameter.name}
              type="number"
              min={parameter.min}
              max={parameter.max}
              step={parameter.step ?? 1}
              disabled={props.readOnly}
              value={Number(parameter.value)}
              onChange={(event) => props.onUpdate(parameter, Number(event.target.value))}
            />
          ) : parameter.type === "boolean" ? (
            <input
              aria-label={parameter.label ?? parameter.name}
              type="checkbox"
              checked={Boolean(parameter.value)}
              disabled={props.readOnly}
              onChange={(event) => props.onUpdate(parameter, event.target.checked)}
            />
          ) : (
            <input
              aria-label={parameter.label ?? parameter.name}
              disabled={props.readOnly}
              value={String(parameter.value)}
              onChange={(event) => props.onUpdate(parameter, event.target.value)}
            />
          )}
        </label>
      ))}
    </section>
  );
}

function Diagnostics({ state }: { state: CadSessionState }) {
  const diagnostics = state.activeRevision?.diagnostics;
  return (
    <section className="diagnostics">
      <h2>Diagnostics</h2>
      <div className="diagnostic-summary">{diagnostics?.ok ? "PASS" : "Needs attention"}</div>
      {(diagnostics?.items ?? []).map((item, index) => (
        <p className={`diagnostic diagnostic-${item.severity}`} key={`${item.message}-${index}`}>
          {item.severity}: {item.message}
        </p>
      ))}
    </section>
  );
}

function Timeline({
  state,
  busy,
  readOnly,
  sourceDirty,
  onActivate,
  onRestore
}: {
  state: CadSessionState;
  busy: boolean;
  readOnly: boolean;
  sourceDirty: boolean;
  onActivate: (revisionId: string) => void;
  onRestore: (revisionId: string) => void;
}) {
  const [diffRevisionId, setDiffRevisionId] = useState<string | null>(null);
  const activeRevision = state.session.revisions.find((revision) => revision.id === state.session.activeRevisionId);
  const diffRevision = diffRevisionId
    ? state.session.revisions.find((revision) => revision.id === diffRevisionId)
    : undefined;
  return (
    <section className="panel">
      <h2>Revisions</h2>
      <ol className="timeline">
        {state.session.revisions.map((revision) => (
          <li className={revision.id === state.session.activeRevisionId ? "active" : ""} key={revision.id}>
            <div className="revision-row-main">
              <span>{revision.id.slice(0, 8)}</span>
              <small>{new Date(revision.createdAt).toLocaleTimeString()}</small>
              <small>{revision.artifactCount} artifacts</small>
              {revision.restoredFromRevisionId ? <small>restored {revision.restoredFromRevisionId.slice(0, 8)}</small> : null}
              {revision.runLinks.length ? <small>{formatRunLinks(revision)}</small> : null}
            </div>
            <div className="revision-actions">
              <button
                aria-label={`Activate revision ${revision.id.slice(0, 8)}`}
                disabled={busy || readOnly || sourceDirty || revision.id === state.session.activeRevisionId}
                onClick={() => onActivate(revision.id)}
                title={sourceDirty ? "Save or discard source edits before switching revisions" : "Activate revision"}
              >
                <SquareMousePointer size={14} />
              </button>
              <button
                aria-label={`Restore revision ${revision.id.slice(0, 8)}`}
                disabled={busy || readOnly}
                onClick={() => onRestore(revision.id)}
                title="Restore revision"
              >
                <RotateCcw size={14} />
              </button>
              <button
                aria-label={`Compare revision ${revision.id.slice(0, 8)}`}
                disabled={!activeRevision || revision.id === activeRevision.id}
                onClick={() => setDiffRevisionId(revision.id)}
                title="Compare with active revision"
              >
                <GitCompare size={14} />
              </button>
            </div>
          </li>
        ))}
      </ol>
      {activeRevision && diffRevision ? (
        <RevisionDiff activeRevision={activeRevision} compareRevision={diffRevision} />
      ) : null}
    </section>
  );
}

function RevisionDiff(props: {
  activeRevision: CadRevisionSummary;
  compareRevision: CadRevisionSummary;
}) {
  const { activeRevision, compareRevision } = props;
  return (
    <div className="revision-diff" data-testid="revision-diff">
      <div>
        <span>Active</span>
        <code>{activeRevision.id.slice(0, 8)}</code>
      </div>
      <div>
        <span>Compare</span>
        <code>{compareRevision.id.slice(0, 8)}</code>
      </div>
      <div>
        <span>Source</span>
        <strong>{activeRevision.sourceHash === compareRevision.sourceHash ? "same hash" : "changed hash"}</strong>
      </div>
      <div>
        <span>Active hash</span>
        <code>{activeRevision.sourceHash.slice(0, 16)}</code>
      </div>
      <div>
        <span>Compare hash</span>
        <code>{compareRevision.sourceHash.slice(0, 16)}</code>
      </div>
      <div>
        <span>Artifacts</span>
        <strong>{formatCountDelta(activeRevision.artifactCount, compareRevision.artifactCount)}</strong>
      </div>
      <div>
        <span>Diagnostics</span>
        <strong>{formatDiagnosticDiff(activeRevision, compareRevision)}</strong>
      </div>
      <div>
        <span>Runs</span>
        <strong>{activeRevision.runLinks.length} / {compareRevision.runLinks.length}</strong>
      </div>
      <div>
        <span>Lineage</span>
        <strong>{formatLineage(compareRevision)}</strong>
      </div>
    </div>
  );
}

function formatCountDelta(activeCount: number, compareCount: number): string {
  const delta = compareCount - activeCount;
  const sign = delta > 0 ? "+" : "";
  return `${activeCount} active / ${compareCount} compare (${sign}${delta})`;
}

function formatDiagnosticDiff(activeRevision: CadRevisionSummary, compareRevision: CadRevisionSummary): string {
  const activeErrors = activeRevision.diagnostics.items.filter((item) => item.severity === "error").length;
  const compareErrors = compareRevision.diagnostics.items.filter((item) => item.severity === "error").length;
  const activeStatus = activeRevision.diagnostics.ok ? "pass" : `${activeErrors} errors`;
  const compareStatus = compareRevision.diagnostics.ok ? "pass" : `${compareErrors} errors`;
  return `${activeStatus} / ${compareStatus}`;
}

function formatRunLinks(revision: CadRevisionSummary): string {
  const inputs = revision.runLinks.filter((link) => link.role === "input").length;
  const outputs = revision.runLinks.filter((link) => link.role === "output").length;
  return `runs ${inputs} in / ${outputs} out`;
}

function formatLineage(revision: CadRevisionSummary): string {
  const parts = [];
  if (revision.parentRevisionId) parts.push(`parent ${revision.parentRevisionId.slice(0, 8)}`);
  if (revision.restoredFromRevisionId) parts.push(`restored ${revision.restoredFromRevisionId.slice(0, 8)}`);
  return parts.join(", ") || "root";
}

function ArtifactList({
  artifacts,
  busy,
  readOnly,
  openedPath,
  integrityResult,
  onExport,
  onOpen,
  onDelete
}: {
  artifacts: CadArtifact[];
  busy: boolean;
  readOnly: boolean;
  openedPath: string | null;
  integrityResult: VerifyArtifactFilesResult | null;
  onExport: (format: "stl" | "metadata", revisionId?: string) => void;
  onOpen: (artifactId: string) => void;
  onDelete: (artifactId: string) => void;
}) {
  const [selectedArtifactId, setSelectedArtifactId] = useState<string | null>(null);
  const selectedArtifact = artifacts.find((artifact) => artifact.id === selectedArtifactId) ?? artifacts[0];
  return (
    <>
      <ul className="artifacts">
        {artifacts.map((artifact) => {
          const status = artifactStatus(artifact, integrityResult);
          const exportFormat = artifactExportFormat(artifact);
          return (
            <li className={`artifact-${status}`} key={artifact.id}>
              <button
                className="artifact-select"
                onClick={() => setSelectedArtifactId(artifact.id)}
                title="Show artifact details"
              >
                <strong>{artifact.kind}.{artifact.format}</strong>
                <span>{artifact.id.slice(0, 8)} · {shortId(artifact.revisionId)} · {formatBytes(artifact.bytes)}</span>
                <small>{status}</small>
              </button>
              <div className="artifact-actions">
                <button
                  aria-label={`Open artifact ${artifact.id.slice(0, 8)}`}
                  disabled={busy || status !== "available"}
                  onClick={() => onOpen(artifact.id)}
                  title={status === "available" ? "Open artifact" : "Artifact cannot be opened until integrity is resolved"}
                >
                  <FolderOpen size={14} />
                </button>
                {exportFormat ? (
                  <button
                    aria-label={`Re-export artifact ${artifact.id.slice(0, 8)}`}
                    disabled={busy || readOnly}
                    onClick={() => onExport(exportFormat, artifact.revisionId)}
                    title="Re-export this artifact format"
                  >
                    <Download size={14} />
                  </button>
                ) : null}
                <button
                  aria-label={`Delete artifact ${artifact.id.slice(0, 8)}`}
                  disabled={busy || readOnly}
                  onClick={() => onDelete(artifact.id)}
                  title="Delete artifact"
                >
                  <Trash2 size={14} />
                </button>
              </div>
            </li>
          );
        })}
      </ul>
      {selectedArtifact ? (
        <ArtifactDetail artifact={selectedArtifact} status={artifactStatus(selectedArtifact, integrityResult)} />
      ) : (
        <p className="empty-state">No artifacts for the selected revision.</p>
      )}
      {openedPath ? <code className="artifact-path">{openedPath}</code> : null}
    </>
  );
}

function artifactExportFormat(artifact: CadArtifact): "stl" | "metadata" | null {
  return artifact.format === "stl" || artifact.format === "metadata" ? artifact.format : null;
}

function ArtifactDetail({ artifact, status }: { artifact: CadArtifact; status: string }) {
  return (
    <section className="artifact-detail" data-testid="artifact-detail">
      <h3>{artifact.kind}.{artifact.format}</h3>
      <dl>
        <div>
          <dt>Status</dt>
          <dd>{status}</dd>
        </div>
        <div>
          <dt>Revision</dt>
          <dd>{artifact.revisionId}</dd>
        </div>
        <div>
          <dt>Created</dt>
          <dd>{new Date(artifact.createdAt).toLocaleString()}</dd>
        </div>
        <div>
          <dt>Bytes</dt>
          <dd>{formatBytes(artifact.bytes)}</dd>
        </div>
        <div>
          <dt>URI</dt>
          <dd>{artifact.uri}</dd>
        </div>
        {artifact.deletedAt ? (
          <div>
            <dt>Deleted</dt>
            <dd>{new Date(artifact.deletedAt).toLocaleString()}</dd>
          </div>
        ) : null}
        {artifact.missingAt ? (
          <div>
            <dt>Missing</dt>
            <dd>{new Date(artifact.missingAt).toLocaleString()}</dd>
          </div>
        ) : null}
      </dl>
      {artifact.metadata && Object.keys(artifact.metadata).length ? (
        <code>{formatPayload(artifact.metadata)}</code>
      ) : null}
    </section>
  );
}

function artifactStatus(artifact: CadArtifact, integrityResult: VerifyArtifactFilesResult | null): "available" | "deleted" | "missing" | "integrity" {
  if (artifact.deletedAt) return "deleted";
  if (artifact.missingAt || integrityResult?.missingArtifactIds.includes(artifact.id)) return "missing";
  if (
    integrityResult?.hashMismatchArtifactIds.includes(artifact.id) ||
    integrityResult?.sizeMismatchArtifactIds.includes(artifact.id) ||
    integrityResult?.corruptMetadataArtifactIds.includes(artifact.id) ||
    integrityResult?.invalidPathArtifactIds.includes(artifact.id)
  ) {
    return "integrity";
  }
  return "available";
}

function formatBytes(value?: number): string {
  if (typeof value !== "number") return "-";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

function replaceUrl(uiUrl: string): void {
  window.history.replaceState({}, "", toHistoryPath(uiUrl, window.location.href));
}

function runtimeMetadata(rendered: OpenscadRenderResult, phase: "preview" | "export"): Record<string, unknown> {
  return {
    runtime: "openscad-wasm",
    sourceLanguage: "openscad",
    sourceHash: rendered.sourceHash,
    parameterHash: rendered.parameterHash,
    stlSha256: rendered.stlSha256,
    stlBytes: rendered.stlBytes.byteLength,
    renderDurationMs: rendered.diagnostics.elapsedMs,
    diagnosticsSource: "openscad-wasm",
    phase
  };
}

function parameterValues(parameters: CadParameter[]) {
  return Object.fromEntries(parameters.map((parameter) => [parameter.name, parameter.value]).sort());
}

async function sha256Hex(input: string | Uint8Array) {
  const bytes = typeof input === "string" ? new TextEncoder().encode(input) : new Uint8Array(input);
  const digest = await crypto.subtle.digest("SHA-256", bytes.buffer as ArrayBuffer);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function base64EncodeUtf8(value: string) {
  return base64EncodeBytes(new TextEncoder().encode(value));
}

function base64EncodeBytes(bytes: Uint8Array) {
  let binary = "";
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(binary);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
