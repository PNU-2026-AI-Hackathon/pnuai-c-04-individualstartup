import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Menu, Trash2, X } from "lucide-react";
import type {
  CadMesh,
  CadDiagnostics,
  CadParameter,
  CadRevision,
  CadSessionListItem,
  CadSessionState,
  PersistRuntimeArtifactInput,
  PersistRuntimeArtifactResult,
  VerifyArtifactFilesResult
} from "./protocol";
import { createCadBackendClient, type ConnectionStatus } from "./backendClient";
import { ArtifactBrowser } from "./components/ArtifactBrowser";
import { DfmSettings } from "./components/DfmSettings";
import {
  isActiveRunStatus,
  latestValidationBatch,
  validationBatchPassed
} from "./components/AgentWorkflow";
import { SessionLogs } from "./components/SessionLogs";
import { SessionBrowser } from "./components/SessionBrowser";
import { SessionRail } from "./components/SessionRail";
import { WorkspacePanel } from "./components/WorkspacePanel";
import {
  base64EncodeUtf8,
  base64EncodeBytes,
  errorMessage,
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
import {
  cancelOpenScadRender,
  createRenderFailureDiagnostics,
  diagnosticsFromOpenScadError,
  isLatestRenderGeneration,
  isOpenScadRenderCanceled,
  logRenderFailureDiagnostics,
  matchesOpenscadPreviewCache,
  nextRenderGeneration,
  renderOpenScadInWorker,
  resetOpenScadRuntimeForSessionSwitch,
  type OpenscadRenderRequest,
  type OpenscadRenderResult,
  type OpenscadRuntimeState
} from "./runtime/openscadRuntime";
import { applyParameterValuesToSource, parameterHashInput, updateParameterDraft } from "./runtime/parameterDraft";
import {
  createAgentStreamState,
  reconcileAgentStreamSnapshot,
  reduceAgentStreamEvent,
  streamingItems
} from "./runtime/agentStream";

export function App() {
  const backend = useMemo(() => createCadBackendClient(), []);
  const [state, setState] = useState<CadSessionState | null>(null);
  const [agentStreamState, setAgentStreamState] = useState(() => createAgentStreamState(""));
  const [source, setSource] = useState("");
  const [sourceDirty, setSourceDirty] = useState(false);
  const [sourceConflict, setSourceConflict] = useState(false);
  const sourceDirtyRef = useRef(false);
  const sourceRef = useRef("");
  const sourceRevisionIdRef = useRef<string | undefined>(undefined);
  const latestSessionIdRef = useRef<string | undefined>(undefined);
  const latestSessionUpdatedAtRef = useRef<string | undefined>(undefined);
  const autoRenderedSessionIdsRef = useRef<Set<string>>(new Set());
  const starterPreviewRenderedSessionIdsRef = useRef<Set<string>>(new Set());
  const [mesh, setMesh] = useState<CadMesh | null>(null);
  const [gcodePreview, setGcodePreview] = useState<{ artifactId: string; contents: string } | null>(null);
  const [runtimeState, setRuntimeState] = useState<OpenscadRuntimeState>("idle");
  const [draftParameters, setDraftParameters] = useState<CadParameter[] | null>(null);
  const draftParametersRef = useRef<CadParameter[] | null>(null);
  const [draftDiagnostics, setDraftDiagnostics] = useState<CadDiagnostics | null>(null);
  const draftRenderGenerationRef = useRef(0);
  const sessionRenderGenerationRef = useRef(0);
  const [agentPrompt, setAgentPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [openedArtifactPath, setOpenedArtifactPath] = useState<string | null>(null);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>("disconnected");
  const [view, setView] = useState<WorkspaceView>(() =>
    workspaceViewFromUrl(window.location.href, window.location.href)
  );
  const [sessionSearch, setSessionSearch] = useState("");
  const [showArchivedSessions, setShowArchivedSessions] = useState(false);
  const [sessionRailOpen, setSessionRailOpen] = useState(false);
  const [sessionList, setSessionList] = useState<CadSessionListItem[]>([]);
  const [locallyDeletedSessionIds, setLocallyDeletedSessionIds] = useState<Set<string>>(new Set());
  const locallyDeletedSessionIdsRef = useRef<Set<string>>(new Set());
  const [pendingDeleteSessionId, setPendingDeleteSessionId] = useState<string | null>(null);
  const [sessionSearchFields, setSessionSearchFields] = useState<string[]>([]);
  const [integrityResult, setIntegrityResult] = useState<VerifyArtifactFilesResult | null>(null);
  const [hiddenStarterSessionIds, setHiddenStarterSessionIds] = useState<Set<string>>(() =>
    loadHiddenStarterSessionIds()
  );
  const [starterSessionHints, setStarterSessionHints] = useState<Set<string>>(new Set());
  const previewCacheRef = useRef<OpenscadRenderResult | null>(null);

  const sessionId = useMemo(() => sessionIdFromUrl(window.location.href, window.location.href), []);
  const activeRevision = state?.activeRevision;
  const activeDraftParameters = draftParameters ?? activeRevision?.parameters ?? [];
  const activeDraftRevision = activeRevision
    ? { ...activeRevision, source, parameters: activeDraftParameters }
    : undefined;
  const workspaceState = useMemo(
    () => state && draftDiagnostics ? stateWithDraftDiagnostics(state, draftDiagnostics) : state,
    [state, draftDiagnostics]
  );
  const previewArtifact = activeRevision?.artifacts.find((artifact) => artifact.kind === "preview-mesh");
  const gcodeArtifact = latestRenderableGcodeArtifact(activeRevision);
  const gcode = gcodePreview && gcodePreview.artifactId === gcodeArtifact?.id
    ? gcodePreview.contents
    : null;
  const activeAgentRun = state?.agentRuns.find((run) => isActiveRunStatus(run.status));
  const sessionArchived = Boolean(state?.session.archivedAt);
  const isStarterSession = Boolean(state && isStarterSessionState(state));
  const visibleSessionList = useMemo(
    () => filterSessionsByDeletedIds(sessionList, locallyDeletedSessionIds),
    [locallyDeletedSessionIds, sessionList]
  );
  const pendingDeleteSession = pendingDeleteSessionId
    ? sessionList.find((session) => session.id === pendingDeleteSessionId) ??
      (state?.session.id === pendingDeleteSessionId
        ? {
            id: state.session.id,
            title: state.session.title
          }
        : undefined)
    : undefined;
  const showStarterOverlay = Boolean(
    state &&
    !hiddenStarterSessionIds.has(state.session.id) &&
    (starterSessionHints.has(state.session.id) || isStarterSessionHeuristic(state))
  );

  const applySessionSnapshot = useCallback(
    (nextState: CadSessionState, options: { forceSource?: boolean } = {}) => {
      const previousSessionId = latestSessionIdRef.current;
      const previousRevisionId = sourceRevisionIdRef.current;
      if (
        previousSessionId === nextState.session.id &&
        latestSessionUpdatedAtRef.current &&
        nextState.session.updatedAt < latestSessionUpdatedAtRef.current
      ) {
        return;
      }
      const nextRevisionId = nextState.session.activeRevisionId;
      if (previousSessionId && (previousSessionId !== nextState.session.id || previousRevisionId !== nextRevisionId)) {
        sessionRenderGenerationRef.current += 1;
        resetOpenScadRuntimeForSessionSwitch();
        previewCacheRef.current = null;
        setRuntimeState("idle");
        setMesh(null);
        setGcodePreview(null);
      }
      latestSessionIdRef.current = nextState.session.id;
      latestSessionUpdatedAtRef.current = nextState.session.updatedAt;
      setAgentStreamState((current) => reconcileAgentStreamSnapshot(current, nextState));
      setState(nextState);
      if (options.forceSource || !sourceDirtyRef.current) {
        setSource(nextState.activeRevision?.source ?? "");
        sourceRef.current = nextState.activeRevision?.source ?? "";
        setSourceDirty(false);
        setSourceConflict(false);
        setDraftParameterState(null);
        setDraftDiagnostics(null);
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
          const booted = await backend.bootSession();
          if (cancelled) return;
          replaceUrl(booted.uiUrl);
          if (booted.shouldUseExampleSession) {
            setStarterSessionHints((previous) => new Set(previous).add(booted.sessionId));
          }
          applySessionSnapshot(booted.state, { forceSource: true });
          await backend.markSessionViewed(booted.sessionId);
          if (booted.shouldAutoRender) {
            await renderAndPersistRevision(booted.sessionId, booted.state.activeRevision);
          }
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
      onStream: (event) => setAgentStreamState((current) => reduceAgentStreamEvent(current, event)),
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
    let cancelled = false;
    if (!gcodeArtifact) {
      setGcodePreview(null);
      return;
    }
    backend.readGcode(gcodeArtifact).then((contents) => {
      if (!cancelled) setGcodePreview({ artifactId: gcodeArtifact.id, contents });
    }).catch((caught) => {
      if (!cancelled) {
        setGcodePreview(null);
        setError(errorMessage(caught));
      }
    });
    return () => {
      cancelled = true;
    };
  }, [backend, gcodeArtifact?.id]);

  useEffect(() => {
    let cancelled = false;
    backend
      .listSessions({ includeArchived: true, query: sessionSearch })
      .then((result) => {
        if (cancelled) return;
        setSessionList(result.sessions);
        setSessionSearchFields(result.searchFields);
      })
      .catch((caught) => setError(errorMessage(caught)));
    return () => {
      cancelled = true;
    };
  }, [backend, sessionSearch, state?.session.updatedAt]);

  useEffect(() => {
    if (!showStarterOverlay || !state?.activeRevision?.runLinks.some((link) => link.role === "output")) return;
    dismissStarterOverlay(state.session.id);
  }, [showStarterOverlay, state?.activeRevision?.id, state?.session.id]);

  useEffect(() => {
    if (!state?.session.id || !isStarterSession || sessionArchived) return;
    if (starterPreviewRenderedSessionIdsRef.current.has(state.session.id)) return;
    renderStarterPreview(state.session.id).catch((caught) => {
      if (isOpenScadRenderCanceled(caught)) return;
      setError(errorMessage(caught));
    });
  }, [isStarterSession, sessionArchived, state?.session.id]);

  useEffect(() => {
    if (!state?.session.id || !draftParameters || sessionArchived) return;
    const generation = nextRenderGeneration(draftRenderGenerationRef.current);
    draftRenderGenerationRef.current = generation;
    const timer = window.setTimeout(() => {
      renderCurrentDraftPreview({ persistIfClean: false, generation }).catch((caught) => {
        if (isOpenScadRenderCanceled(caught)) return;
        if (!isLatestRenderGeneration(draftRenderGenerationRef.current, generation)) return;
        const diagnostics = diagnosticsFromOpenScadError(caught);
        if (diagnostics) setDraftDiagnostics(diagnostics);
        setError(errorMessage(caught));
      });
    }, 400);
    return () => {
      window.clearTimeout(timer);
      draftRenderGenerationRef.current = nextRenderGeneration(draftRenderGenerationRef.current);
      cancelOpenScadRender();
    };
  }, [draftParameters, sessionArchived, state?.session.id, source]);

  async function saveSource() {
    if (!state || sessionArchived) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      dismissStarterOverlay(state.session.id);
      const result = await backend.updateModelSource({
        sessionId: state.session.id,
        sourceLanguage: "openscad",
        source: sourceRef.current,
        parentRevisionId: state.session.activeRevisionId,
        parameters: currentDraftParameters()
      });
      applySessionSnapshot(result.state, { forceSource: true });
    });
  }

  async function updateParameter(parameter: CadParameter, value: CadParameter["value"]) {
    if (!state || sessionArchived) return;
    dismissStarterOverlay(state.session.id);
    let nextParameters: CadParameter[];
    try {
      nextParameters = updateParameterDraft(currentDraftParameters(), parameter.name, value);
    } catch (caught) {
      const diagnostics = createRenderFailureDiagnostics({
        origin: "parameter-draft",
        code: errorCode(caught),
        message: errorMessage(caught),
        sessionId: state.session.id,
        revisionId: state.activeRevision?.id,
        sourceHash: state.activeRevision?.sourceHash
      });
      logRenderFailureDiagnostics(diagnostics);
      setDraftDiagnostics(diagnostics);
      setRuntimeState("failed");
      setError(errorMessage(caught));
      return;
    }
    setDraftParameterState(nextParameters);
    setSource((previous) => {
      const nextSource = applyParameterValuesToSource(previous, nextParameters);
      sourceRef.current = nextSource;
      return nextSource;
    });
    setDraftDiagnostics(null);
    setSourceDirty(true);
    sourceDirtyRef.current = true;
  }

  async function startAgentRun(promptOverride?: string, retryOfRunId?: string) {
    if (!state || sessionArchived) return;
    const prompt = (promptOverride ?? agentPrompt).trim();
    if (!prompt) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      dismissStarterOverlay(state.session.id);
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

  async function startNewAgentConversation() {
    if (!state || sessionArchived || activeAgentRun) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.startNewAgentConversation(state.session.id);
      applySessionSnapshot(result.state);
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
        const revision = await ensureSavedRevisionForExport(revisionId);
        const rendered = await cachedOrRenderedStl(revision);
        const persisted = await persistRuntimeArtifactWithDiagnostics({
          sessionId: state.session.id,
          revisionId: revision.id,
          kind: "stl",
          format,
          contentsBase64: base64EncodeBytes(rendered.stlBytes),
          diagnostics: rendered.diagnostics,
          metadata: runtimeMetadata(rendered, "export")
        }, rendered);
        applySessionSnapshot(persisted.state);
        return;
      }
      const result = await backend.exportArtifact({ sessionId: state.session.id, revisionId, format });
      applySessionSnapshot(result.state);
    });
  }

  async function ensureSavedRevisionForExport(revisionId = state?.session.activeRevisionId): Promise<CadRevision> {
    if (!state) throw new Error("No active session is loaded.");
    if (sourceDirty && revisionId === state.session.activeRevisionId) {
      const result = await backend.updateModelSource({
        sessionId: state.session.id,
        sourceLanguage: "openscad",
        source: sourceRef.current,
        parentRevisionId: state.session.activeRevisionId,
        parameters: currentDraftParameters()
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
    const sourceHash = await sha256Hex(applyParameterValuesToSource(revision.source, revision.parameters));
    const parameterHash = await sha256Hex(parameterHashInput(revision.parameters));
    const cache = previewCacheRef.current;
    if (state && matchesOpenscadPreviewCache(cache, {
      sessionId: state.session.id,
      revisionId: revision.id,
      sourceHash,
      parameterHash
    })) {
      return cache;
    }
    if (!state) throw new Error("No active session is loaded.");
    const renderGeneration = sessionRenderGenerationRef.current;
    const renderRequest = await renderRequestFor(state.session.id, revision.id, revision.source, revision.parameters);
    if (!(await isCurrentRenderTarget(renderRequest, renderGeneration))) {
      throw new Error("The STL render target changed before export started.");
    }
    const rendered = await renderOpenScadInWorker(renderRequest, setRuntimeStateForRenderTarget(renderRequest, renderGeneration));
    if (!(await isCurrentRenderTarget(renderRequest, renderGeneration))) {
      throw new Error("The STL render target changed before export completed.");
    }
    previewCacheRef.current = rendered;
    setMesh(rendered.mesh);
    return rendered;
  }

  async function openArtifact(artifactId: string) {
    await runBusy(async () => {
      const result = await backend.openArtifact(artifactId);
      setOpenedArtifactPath(result.path);
    });
  }

  async function copyArtifactPath(artifactId: string) {
    await runBusy(async () => {
      const result = await backend.openArtifact(artifactId);
      await navigator.clipboard.writeText(result.path);
      setOpenedArtifactPath(result.path);
    });
  }

  async function revealArtifact(artifactId: string) {
    await runBusy(async () => {
      const result = await backend.revealArtifact(artifactId);
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
    setSessionList(result.sessions);
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

  function requestDeleteSession(sessionId: string) {
    setPendingDeleteSessionId(sessionId);
  }

  async function confirmDeleteSession() {
    if (!pendingDeleteSessionId) return;
    const sessionId = pendingDeleteSessionId;
    setPendingDeleteSessionId(null);
    console.info("[cadastrophe:delete-session] handler entered", { sessionId });
    markSessionLocallyDeleted(sessionId);
    console.info("[cadastrophe:delete-session] marked locally deleted", { sessionId });
    setSessionList((sessions) => sessions.filter((session) => session.id !== sessionId));
    console.info("[cadastrophe:delete-session] filtered session list optimistically", { sessionId });
    await runBusy(async () => {
      console.info("[cadastrophe:delete-session] invoking backend delete", { sessionId });
      const deleted = await backend.deleteSession(sessionId).catch(async (caught) => {
        console.error("[cadastrophe:delete-session] backend delete failed", { sessionId, error: caught });
        restoreLocallyDeletedSession(sessionId);
        console.warn("[cadastrophe:delete-session] restored local delete marker after backend failure", { sessionId });
        await refreshSessionList().catch((refreshError) => {
          console.error("[cadastrophe:delete-session] refresh after delete failure also failed", {
            sessionId,
            error: refreshError
          });
        });
        throw caught;
      });
      console.info("[cadastrophe:delete-session] backend delete succeeded", { sessionId, deleted });
      setOpenedArtifactPath(null);
      await refreshSessionList();
      console.info("[cadastrophe:delete-session] refreshed session list after delete", { sessionId });
      if (state?.session.id !== sessionId) {
        console.info("[cadastrophe:delete-session] deleted session was not active session", {
          sessionId,
          activeSessionId: state?.session.id
        });
        return;
      }
      const nextSessionId =
        deleted.currentSessionId ??
        filterLocallyDeletedSessions((await backend.listSessions({ includeArchived: false })).sessions)[0]?.id;
      console.info("[cadastrophe:delete-session] resolved replacement session", { sessionId, nextSessionId });
      if (nextSessionId) {
        const nextState = await loadSession(nextSessionId, { forceSource: true });
        await backend.markSessionViewed(nextSessionId);
        navigateTo(view, nextState.session.id);
        console.info("[cadastrophe:delete-session] navigated to replacement session", {
          sessionId,
          nextSessionId
        });
        return;
      }
      const created = await backend.createSession({ title: "Cadastrophe review" });
      applySessionSnapshot(created.state, { forceSource: true });
      await backend.markSessionViewed(created.sessionId);
      await refreshSessionList();
      navigateTo(view, created.sessionId);
      console.info("[cadastrophe:delete-session] created replacement session", {
        sessionId,
        createdSessionId: created.sessionId
      });
    });
  }

  function filterLocallyDeletedSessions(sessions: CadSessionListItem[]) {
    return filterSessionsByDeletedIds(sessions, locallyDeletedSessionIdsRef.current);
  }

  function markSessionLocallyDeleted(sessionId: string) {
    setLocallyDeletedSessionIds((previous) => {
      const next = new Set(previous);
      next.add(sessionId);
      locallyDeletedSessionIdsRef.current = next;
      return next;
    });
  }

  function restoreLocallyDeletedSession(sessionId: string) {
    setLocallyDeletedSessionIds((previous) => {
      const next = new Set(previous);
      next.delete(sessionId);
      locallyDeletedSessionIdsRef.current = next;
      return next;
    });
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
    if (state?.session.id) dismissStarterOverlay(state.session.id);
    sourceRef.current = value;
    setSource(value);
    setDraftDiagnostics(null);
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

  function setDraftParameterState(parameters: CadParameter[] | null) {
    draftParametersRef.current = parameters;
    setDraftParameters(parameters);
  }

  function currentDraftParameters(): CadParameter[] {
    return draftParametersRef.current ?? state?.activeRevision?.parameters ?? [];
  }

  async function renderCurrentDraftPreview({
    persistIfClean,
    generation
  }: {
    persistIfClean: boolean;
    generation?: number;
  }) {
    if (!state) return;
    const sessionId = state.session.id;
    const revisionId = state.activeRevision?.id ?? EMPTY_DRAFT_REVISION_ID;
    const renderGeneration = sessionRenderGenerationRef.current;
    const renderSource = sourceRef.current;
    const renderParameters = currentDraftParameters();
    let renderRequest: OpenscadRenderRequest | undefined;
    let rendered: OpenscadRenderResult;
    try {
      renderRequest = await renderRequestFor(sessionId, revisionId, renderSource, renderParameters);
      if (!(await isCurrentRenderTarget(renderRequest, renderGeneration))) return;
      rendered = await renderOpenScadInWorker(renderRequest, setRuntimeStateForRenderTarget(renderRequest, renderGeneration));
    } catch (caught) {
      if (isOpenScadRenderCanceled(caught)) return;
      if (!isLatestRenderGeneration(draftRenderGenerationRef.current, generation)) return;
      if (renderRequest && !(await isCurrentRenderTarget(renderRequest, renderGeneration))) return;
      const diagnostics = diagnosticsFromOpenScadError(caught);
      if (diagnostics) {
        setDraftDiagnostics(diagnostics);
      } else {
        setLocalRenderFailureDiagnostics(caught, "parameter-draft", sessionId, revisionId);
      }
      throw caught;
    }
    if (!isLatestRenderGeneration(draftRenderGenerationRef.current, generation)) return;
    if (!(await isCurrentRenderTarget(renderRequest, renderGeneration))) return;
    previewCacheRef.current = rendered;
    setMesh(rendered.mesh);
    setDraftDiagnostics(rendered.diagnostics);
    if (!state.activeRevision || !persistIfClean || sourceDirtyRef.current) return;
    const persisted = await persistPreviewMesh(sessionId, revisionId, rendered);
    if (await isCurrentRenderTarget(renderRequest, renderGeneration)) {
      applySessionSnapshot(persisted.state);
    }
  }

  async function persistPreviewMesh(sessionId: string, revisionId: string, rendered: OpenscadRenderResult) {
    return persistRuntimeArtifactWithDiagnostics({
        sessionId,
        revisionId,
        kind: "preview-mesh",
        format: "json",
        contentsBase64: base64EncodeUtf8(JSON.stringify(rendered.mesh)),
        diagnostics: rendered.diagnostics,
        metadata: runtimeMetadata(rendered, "preview")
      }, rendered);
  }

  async function persistRuntimeArtifactWithDiagnostics(
    input: PersistRuntimeArtifactInput,
    rendered: OpenscadRenderResult
  ): Promise<PersistRuntimeArtifactResult> {
    try {
      return await backend.persistRuntimeArtifact(input);
    } catch (caught) {
      const diagnostics = createRenderFailureDiagnostics({
        origin: "tauri-persistence",
        code: errorCode(caught),
        message: errorMessage(caught),
        sessionId: input.sessionId,
        revisionId: input.revisionId,
        sourceHash: rendered.sourceHash,
        parameterHash: rendered.parameterHash,
        elapsedMs: rendered.diagnostics.elapsedMs
      });
      logRenderFailureDiagnostics(diagnostics);
      setRuntimeState("failed");
      setDraftDiagnostics(diagnostics);
      throw caught;
    }
  }

  function setLocalRenderFailureDiagnostics(
    caught: unknown,
    origin: "parameter-draft" | "stale-render",
    sessionId: string,
    revisionId: string
  ) {
    const diagnostics = createRenderFailureDiagnostics({
      origin,
      code: errorCode(caught),
      message: errorMessage(caught),
      sessionId,
      revisionId,
      sourceHash: state?.activeRevision?.sourceHash
    });
    logRenderFailureDiagnostics(diagnostics);
    setRuntimeState("failed");
    setDraftDiagnostics(diagnostics);
  }

  async function renderAndPersistRevision(sessionId: string, revision?: CadRevision) {
    if (!revision || autoRenderedSessionIdsRef.current.has(sessionId)) return;
    autoRenderedSessionIdsRef.current.add(sessionId);
    const renderGeneration = sessionRenderGenerationRef.current;
    const renderRequest = await renderRequestFor(sessionId, revision.id, revision.source, revision.parameters);
    if (!(await isCurrentRenderTarget(renderRequest, renderGeneration))) return;
    try {
      const rendered = await renderOpenScadInWorker(renderRequest, setRuntimeStateForRenderTarget(renderRequest, renderGeneration));
      if (!(await isCurrentRenderTarget(renderRequest, renderGeneration))) return;
      previewCacheRef.current = rendered;
      setMesh(rendered.mesh);
      setDraftDiagnostics(rendered.diagnostics);
      const persisted = await persistPreviewMesh(sessionId, revision.id, rendered);
      if (await isCurrentRenderTarget(renderRequest, renderGeneration)) {
        applySessionSnapshot(persisted.state);
      }
    } catch (caught) {
      if (isOpenScadRenderCanceled(caught)) return;
      if (!(await isCurrentRenderTarget(renderRequest, renderGeneration))) return;
      const diagnostics = diagnosticsFromOpenScadError(caught);
      if (diagnostics) setDraftDiagnostics(diagnostics);
      setError(errorMessage(caught));
    }
  }

  async function renderStarterPreview(sessionId: string) {
    starterPreviewRenderedSessionIdsRef.current.add(sessionId);
    const renderGeneration = sessionRenderGenerationRef.current;
    const renderRequest = await renderRequestFor(
      sessionId,
      STARTER_PREVIEW_REVISION_ID,
      STARTER_SAMPLE_SOURCE,
      []
    );
    if (!isCurrentStarterPreviewTarget(sessionId, renderGeneration)) return;
    try {
      const rendered = await renderOpenScadInWorker(
        renderRequest,
        setRuntimeStateForStarterTarget(sessionId, renderGeneration)
      );
      if (!isCurrentStarterPreviewTarget(sessionId, renderGeneration)) return;
      setMesh(rendered.mesh);
    } catch (caught) {
      if (isOpenScadRenderCanceled(caught)) return;
      if (!isCurrentStarterPreviewTarget(sessionId, renderGeneration)) return;
      throw caught;
    }
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

  function dismissStarterOverlay(sessionId = state?.session.id) {
    if (!sessionId || hiddenStarterSessionIds.has(sessionId)) return;
    setHiddenStarterSessionIds((previous) => {
      const next = new Set(previous);
      next.add(sessionId);
      saveHiddenStarterSessionIds(next);
      return next;
    });
  }

  async function renderRequestFor(
    sessionId: string,
    revisionId: string,
    renderSource: string,
    renderParameters: CadParameter[]
  ): Promise<OpenscadRenderRequest> {
    return {
      sessionId,
      revisionId,
      source: renderSource,
      parameters: renderParameters,
      sourceHash: await sha256Hex(applyParameterValuesToSource(renderSource, renderParameters)),
      parameterHash: await sha256Hex(parameterHashInput(renderParameters))
    };
  }

  function setRuntimeStateForRenderTarget(
    request: OpenscadRenderRequest,
    generation: number
  ): (nextState: OpenscadRuntimeState) => void {
    return (nextState) => {
      if (!isCurrentRenderGeneration(request, generation)) return;
      setRuntimeState(nextState);
    };
  }

  function isCurrentRenderGeneration(request: OpenscadRenderRequest, generation: number): boolean {
    const currentRevisionId = sourceRevisionIdRef.current ?? EMPTY_DRAFT_REVISION_ID;
    return (
      sessionRenderGenerationRef.current === generation &&
      latestSessionIdRef.current === request.sessionId &&
      currentRevisionId === request.revisionId
    );
  }

  function setRuntimeStateForStarterTarget(
    sessionId: string,
    generation: number
  ): (nextState: OpenscadRuntimeState) => void {
    return (nextState) => {
      if (!isCurrentStarterPreviewTarget(sessionId, generation)) return;
      setRuntimeState(nextState);
    };
  }

  function isCurrentStarterPreviewTarget(sessionId: string, generation: number): boolean {
    return (
      sessionRenderGenerationRef.current === generation &&
      latestSessionIdRef.current === sessionId &&
      sourceRevisionIdRef.current === undefined &&
      !sourceDirtyRef.current
    );
  }

  async function isCurrentRenderTarget(
    request: OpenscadRenderRequest,
    generation: number
  ): Promise<boolean> {
    if (!isCurrentRenderGeneration(request, generation)) return false;
    const currentParameters = currentDraftParameters();
    const sourceHash = await sha256Hex(applyParameterValuesToSource(sourceRef.current, currentParameters));
    if (sourceHash !== request.sourceHash) return false;
    const parameterHash = await sha256Hex(parameterHashInput(currentParameters));
    return parameterHash === request.parameterHash;
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
      <SessionRail
        sessions={visibleSessionList}
        activeSessionId={state.session.id}
        query={sessionSearch}
        showArchived={showArchivedSessions}
        busy={busy}
        open={sessionRailOpen}
        view={view}
        revisionState={workspaceState ?? state}
        revisionsReadOnly={sessionArchived}
        sourceDirty={sourceDirty}
        onQueryChange={setSessionSearch}
        onShowArchivedChange={setShowArchivedSessions}
        onCreateSession={createNewSession}
        onOpenSession={openSession}
        onArchiveChange={setSessionArchived}
        onRename={renameSession}
        onDuplicate={duplicateSession}
        onDelete={requestDeleteSession}
        onNavigate={navigateTo}
        onActivateRevision={setActiveRevision}
        onRestoreRevision={restoreRevision}
        onClose={() => setSessionRailOpen(false)}
      />
      {sessionRailOpen ? (
        <button className="rail-backdrop" aria-label="Close session rail" onClick={() => setSessionRailOpen(false)} />
      ) : null}
      {pendingDeleteSessionId ? (
        <div className="confirm-dialog-backdrop" role="presentation" onMouseDown={() => setPendingDeleteSessionId(null)}>
          <section
            aria-labelledby="delete-session-title"
            aria-modal="true"
            className="confirm-dialog"
            onKeyDown={(event) => {
              if (event.key === "Escape") setPendingDeleteSessionId(null);
            }}
            onMouseDown={(event) => event.stopPropagation()}
            role="dialog"
            tabIndex={-1}
          >
            <header>
              <h2 id="delete-session-title">Delete session?</h2>
              <button
                aria-label="Cancel delete session"
                autoFocus
                disabled={busy}
                onClick={() => setPendingDeleteSessionId(null)}
                title="Cancel delete session"
              >
                <X size={16} />
              </button>
            </header>
            <p>{pendingDeleteSession?.title ?? "Untitled CAD session"}</p>
            <code>{pendingDeleteSessionId}</code>
            <div className="confirm-dialog-actions">
              <button disabled={busy} onClick={() => setPendingDeleteSessionId(null)} title="Cancel delete session">
                Cancel
              </button>
              <button className="danger" disabled={busy} onClick={confirmDeleteSession} title="Delete session">
                <Trash2 size={16} /> Delete
              </button>
            </div>
          </section>
        </div>
      ) : null}

      <div className={view === "workspace" ? "workspace-main workspace-main-model" : "workspace-main"}>
        <header className="topbar">
          <button className="rail-menu-button" onClick={() => setSessionRailOpen(true)} title="Open sessions">
            <Menu size={17} />
          </button>
          <div className="topbar-context">
            <h1>{state.session.title}</h1>
            <p><span>Session</span> {state.session.id}</p>
          </div>
          <div className="status-group">
            {topbarStatusLabel(workspaceState ?? state, Boolean(activeAgentRun), sessionArchived) ? (
              <div className={`status status-${topbarStatusTone(workspaceState ?? state, Boolean(activeAgentRun), sessionArchived)}`} data-testid="session-status">
                {topbarStatusLabel(workspaceState ?? state, Boolean(activeAgentRun), sessionArchived)}
              </div>
            ) : null}
            {connectionStatus !== "connected" ? (
              <div className={`connection connection-${connectionStatus}`}>{connectionStatus}</div>
            ) : null}
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

        {sessionArchived ? (
          <div className="warning" role="status">
            <span>Archived session. Unarchive it before editing.</span>
            <button onClick={() => setSessionArchived(state.session.id, false)} disabled={busy} title="Unarchive session">
              Unarchive
            </button>
          </div>
        ) : null}

        {view === "workspace" ? (
          <WorkspacePanel
            state={workspaceState ?? state}
            mesh={mesh}
            gcode={gcode}
            gcodeArtifactId={gcodeArtifact?.id}
            gcodeBedShape={gcodeArtifact?.metadata?.bedShape}
            runtimeState={runtimeState}
            busy={busy}
            sessionArchived={sessionArchived}
            source={source}
            starterSource={isStarterSession ? STARTER_SAMPLE_SOURCE : undefined}
            sourceDirty={sourceDirty}
            showStarterOverlay={showStarterOverlay}
            activeRevision={activeDraftRevision}
            activeRun={activeAgentRun}
            agentPrompt={agentPrompt}
            agentStreams={streamingItems(agentStreamState)}
            onSaveSource={saveSource}
            onEditSource={editSource}
            onDismissStarterOverlay={dismissStarterOverlay}
            onPromptChange={setAgentPrompt}
            onStartRun={() => startAgentRun()}
            onStartNewConversation={startNewAgentConversation}
            onRetryRun={(run) => startAgentRun(run.prompt, run.id)}
            onCancelRun={cancelAgentRun}
            onUpdateParameter={updateParameter}
            onExport={exportArtifact}
            onOpenFullHistory={() => navigateTo("logs")}
          />
        ) : null}

        {view === "sessions" ? (
          <SessionBrowser
            sessions={visibleSessionList}
            activeSessionId={state.session.id}
            query={sessionSearch}
            searchFields={sessionSearchFields}
            busy={busy}
            onQueryChange={setSessionSearch}
            onOpen={openSession}
            onArchiveChange={setSessionArchived}
            onRename={renameSession}
            onDuplicate={duplicateSession}
            onDelete={requestDeleteSession}
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
            validationEvaluations={state.validationEvaluations}
            validationBatches={state.validationBatches}
            validationChecks={state.validationChecks}
          />
        ) : null}

        {view === "settings" ? <DfmSettings backend={backend} /> : null}
      </div>
    </main>
  );
}

const STARTER_OVERLAY_KEY = "cadastrophe.hiddenStarterOverlaySessions";
const EMPTY_DRAFT_REVISION_ID = "__empty-draft__";
const STARTER_PREVIEW_REVISION_ID = "__starter-preview__";
const STARTER_SAMPLE_SOURCE = `width = 32; // @param min=8 max=80 step=1 label=Width
depth = 24; // @param min=8 max=80 step=1 label=Depth
height = 12; // @param min=4 max=60 step=1 label=Height

cube([width, depth, height]);
translate([24, 0, height]) cylinder(h=height * 2, r=6);
translate([-24, 0, 12]) sphere(r=8);
`;

function loadHiddenStarterSessionIds(): Set<string> {
  try {
    const values = JSON.parse(localStorage.getItem(STARTER_OVERLAY_KEY) ?? "[]");
    return new Set(Array.isArray(values) ? values.filter((value) => typeof value === "string") : []);
  } catch {
    return new Set();
  }
}

function saveHiddenStarterSessionIds(sessionIds: Set<string>) {
  localStorage.setItem(STARTER_OVERLAY_KEY, JSON.stringify([...sessionIds]));
}

function isStarterSessionHeuristic(state: CadSessionState): boolean {
  if (isStarterSessionState(state)) return true;
  return (
    state.conversation.length === 0 &&
    state.agentRuns.length === 0 &&
    state.session.revisions.length <= 1 &&
    Boolean(state.activeRevision?.source.trim()) &&
    (state.session.title === "Cadastrophe review" || !state.session.title)
  );
}

function isStarterSessionState(state: CadSessionState): boolean {
  return (
    state.conversation.length === 0 &&
    state.agentRuns.length === 0 &&
    state.session.revisions.length === 0 &&
    !state.session.activeRevisionId &&
    !state.activeRevision
  );
}

function stateWithDraftDiagnostics(state: CadSessionState, diagnostics: CadDiagnostics): CadSessionState {
  const activeRevisionId = state.session.activeRevisionId;
  return {
    ...state,
    session: {
      ...state.session,
      revisions: state.session.revisions.map((revision) => (
        revision.id === activeRevisionId ? { ...revision, diagnostics } : revision
      ))
    },
    activeRevision: state.activeRevision ? { ...state.activeRevision, diagnostics } : state.activeRevision
  };
}

export function filterSessionsByDeletedIds(
  sessions: CadSessionListItem[],
  deletedSessionIds: Set<string>
): CadSessionListItem[] {
  if (deletedSessionIds.size === 0) return sessions;
  return sessions.filter((session) => !deletedSessionIds.has(session.id));
}

export function latestRenderableGcodeArtifact(revision?: CadRevision) {
  return revision?.artifacts.reduce<(typeof revision.artifacts)[number] | undefined>((latest, artifact) => {
    if (
      artifact.kind !== "gcode" ||
      artifact.format !== "gcode" ||
      artifact.deletedAt ||
      artifact.missingAt
    ) {
      return latest;
    }
    return !latest || artifact.createdAt > latest.createdAt ? artifact : latest;
  }, undefined);
}

function topbarStatusLabel(state: CadSessionState, hasActiveRun: boolean, archived: boolean): string | null {
  if (archived) return "Archived";
  const run = state.agentRuns.at(-1);
  const batch = run
    ? latestValidationBatch(state.validationBatches, run.id, run.outputRevisionId)
    : undefined;
  if (hasActiveRun) return "Agent running";
  if (state.activeRevision?.diagnostics.ok === false) return "Needs source fix";
  if (batch?.status === "failed") return "Validation failed";
  if (batch?.status === "succeeded") {
    return validationBatchPassed(batch) ? "Finalized" : "Needs refinement";
  }
  if (state.workflow.outerIterations.some((iteration) => iteration.passed)) return "Finalized";
  if (state.session.status === "rendering") return "Rendering";
  if (state.session.status === "failed") return "Session failed";
  return null;
}

function topbarStatusTone(state: CadSessionState, hasActiveRun: boolean, archived: boolean): string {
  if (archived) return "archived";
  const run = state.agentRuns.at(-1);
  const batch = run
    ? latestValidationBatch(state.validationBatches, run.id, run.outputRevisionId)
    : undefined;
  if (batch?.status === "failed"
    || (batch?.status === "succeeded" && !validationBatchPassed(batch))) return "failed";
  if (state.activeRevision?.diagnostics.ok === false || state.session.status === "failed") return "failed";
  if (hasActiveRun || state.session.status === "rendering") return "rendering";
  return "finalized";
}

function errorCode(error: unknown): string | number | undefined {
  if (typeof error === "object" && error && "code" in error) {
    const code = (error as { code?: unknown }).code;
    if (typeof code === "string" || typeof code === "number") return code;
  }
  return /(?:error|code)[:\s]+(\d+)/i.exec(errorMessage(error))?.[1];
}
