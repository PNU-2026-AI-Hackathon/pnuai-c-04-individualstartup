import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Download,
  Play,
  RefreshCcw,
  Save,
  Send,
  X
} from "lucide-react";
import type {
  CadAgentRun,
  CadArtifact,
  CadConversationMessage,
  CadMesh,
  CadParameter,
  CadSessionState
} from "./protocol";
import { createCadBackendClient, type ConnectionStatus } from "./backendClient";
import { MeshPreview } from "./MeshPreview";
import { toHistoryPath } from "./navigation";

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
  const [agentPrompt, setAgentPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [connectionStatus, setConnectionStatus] = useState<ConnectionStatus>("disconnected");

  const sessionId = useMemo(() => sessionIdFromPath(), []);
  const activeRevision = state?.activeRevision;
  const previewArtifact = activeRevision?.artifacts.find((artifact) => artifact.kind === "preview-mesh");
  const activeAgentRun = state?.agentRuns.find((run) => isActiveRunStatus(run.status));

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

  async function saveSource() {
    if (!state) return;
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
    if (!state) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.renderPreview({ sessionId: state.session.id });
      applySessionSnapshot(result.state);
    });
  }

  async function updateParameter(parameter: CadParameter, value: CadParameter["value"]) {
    if (!state) return;
    await resyncIfDisconnected();
    const result = await backend.updateParameters({
      sessionId: state.session.id,
      values: { [parameter.name]: value }
    });
    applySessionSnapshot(result);
  }

  async function startAgentRun(promptOverride?: string) {
    if (!state) return;
    const prompt = (promptOverride ?? agentPrompt).trim();
    if (!prompt) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      await backend.createAgentRun({
        sessionId: state.session.id,
        prompt,
        revisionId: state.session.activeRevisionId
      });
      if (!promptOverride) setAgentPrompt("");
      await loadSession(state.session.id);
    });
  }

  async function cancelAgentRun(runId: string) {
    if (!state) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.cancelAgentRun({ sessionId: state.session.id, runId });
      applySessionSnapshot(result.state);
    });
  }

  async function exportArtifact(format: "stl" | "metadata") {
    if (!state) return;
    await runBusy(async () => {
      await resyncIfDisconnected();
      const result = await backend.exportArtifact({ sessionId: state.session.id, format });
      applySessionSnapshot(result.state);
    });
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

      <section className="workspace">
        <div className="preview-pane">
          <div className="pane-toolbar">
            <h2>Preview</h2>
            <button onClick={renderPreview} disabled={busy} title="Render preview">
              <Play size={16} /> Render
            </button>
          </div>
          <MeshPreview mesh={mesh} />
          <Diagnostics state={state} />
        </div>

        <div className="editor-pane">
          <div className="pane-toolbar">
            <h2>OpenSCAD Source</h2>
            <button onClick={saveSource} disabled={busy || !sourceDirty} title="Save source revision">
              <Save size={16} /> Save
            </button>
          </div>
          <textarea
            data-testid="source-editor"
            value={source}
            onChange={(event) => editSource(event.target.value)}
            spellCheck={false}
          />
        </div>

        <aside className="side-pane">
          <AgentWorkspace
            conversation={state.conversation}
            runs={state.agentRuns}
            prompt={agentPrompt}
            busy={busy}
            activeRun={activeAgentRun}
            onPromptChange={setAgentPrompt}
            onStartRun={() => startAgentRun()}
            onRetryRun={(run) => startAgentRun(run.prompt)}
            onCancelRun={cancelAgentRun}
          />
          <Parameters revision={activeRevision} onUpdate={updateParameter} />
          <Timeline state={state} />
          <section className="panel">
            <h2>Export</h2>
            <div className="button-row">
              <button onClick={() => exportArtifact("stl")} disabled={busy} title="Export STL">
                <Download size={16} /> STL
              </button>
              <button onClick={() => exportArtifact("metadata")} disabled={busy} title="Export metadata">
                <RefreshCcw size={16} /> Metadata
              </button>
            </div>
            <ArtifactList artifacts={activeRevision?.artifacts ?? []} />
          </section>
        </aside>
      </section>
    </main>
  );
}

function AgentWorkspace(props: {
  conversation: CadConversationMessage[];
  runs: CadAgentRun[];
  prompt: string;
  busy: boolean;
  activeRun?: CadAgentRun;
  onPromptChange: (value: string) => void;
  onStartRun: () => void;
  onRetryRun: (run: CadAgentRun) => void;
  onCancelRun: (runId: string) => void;
}) {
  const latestRun = props.runs.at(-1);
  const timeline = buildConversationTimeline(props.conversation, props.runs);
  const promptDisabled = props.busy || Boolean(props.activeRun);
  return (
    <section className="panel agent-workspace">
      <div className="panel-heading">
        <h2>Codex Agent</h2>
        <RunStatusBadge run={props.activeRun ?? latestRun} />
      </div>
      {props.activeRun?.activeStep ? (
        <div className="active-step" data-testid="active-step">
          {props.activeRun.activeStep.replaceAll("_", " ")}
        </div>
      ) : null}
      <ol className="conversation" data-testid="conversation-timeline">
        {timeline.map((item) => (
          <li className={`conversation-item conversation-${item.kind}`} key={item.id}>
            <span>{item.label}</span>
            <p>{item.content}</p>
            <small>{new Date(item.createdAt).toLocaleTimeString()}</small>
          </li>
        ))}
      </ol>
      <textarea
        className="small-textarea"
        aria-label="Ask Codex agent"
        data-testid="agent-prompt"
        value={props.prompt}
        onChange={(event) => props.onPromptChange(event.target.value)}
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
          disabled={props.busy || !props.activeRun}
          title="Cancel agent run"
        >
          <X size={16} /> Cancel
        </button>
      </div>
      {latestRun?.status === "failed" || latestRun?.status === "cancelled" ? (
        <button
          data-testid="retry-agent-run"
          onClick={() => props.onRetryRun(latestRun)}
          disabled={props.busy || Boolean(props.activeRun)}
          title="Retry agent run"
        >
          <RefreshCcw size={16} /> Retry
        </button>
      ) : null}
    </section>
  );
}

function RunStatusBadge({ run }: { run?: CadAgentRun }) {
  const status = run?.status ?? "idle";
  return (
    <div className={`run-status run-status-${status}`} data-testid="agent-run-status">
      {status.replaceAll("_", " ")}
    </div>
  );
}

function buildConversationTimeline(conversation: CadConversationMessage[], runs: CadAgentRun[]) {
  const items = [
    ...conversation.map((message) => ({
      id: message.id,
      createdAt: message.createdAt,
      kind: message.role,
      label: message.role,
      content: message.content
    })),
    ...runs.map((run) => ({
      id: run.id,
      createdAt: run.updatedAt,
      kind: "run",
      label: `run ${run.status.replaceAll("_", " ")}`,
      content: run.error ?? run.activeStep ?? run.prompt
    }))
  ];
  return items.sort((left, right) => left.createdAt.localeCompare(right.createdAt));
}

function isActiveRunStatus(status: CadAgentRun["status"]): boolean {
  return status === "queued" || status === "running" || status === "waiting_for_user";
}

function Parameters(props: {
  revision: CadSessionState["activeRevision"];
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
              value={Number(parameter.value)}
              onChange={(event) => props.onUpdate(parameter, Number(event.target.value))}
            />
          ) : parameter.type === "boolean" ? (
            <input
              aria-label={parameter.label ?? parameter.name}
              type="checkbox"
              checked={Boolean(parameter.value)}
              onChange={(event) => props.onUpdate(parameter, event.target.checked)}
            />
          ) : (
            <input
              aria-label={parameter.label ?? parameter.name}
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

function Timeline({ state }: { state: CadSessionState }) {
  return (
    <section className="panel">
      <h2>Revisions</h2>
      <ol className="timeline">
        {state.session.revisions.map((revision) => (
          <li className={revision.id === state.session.activeRevisionId ? "active" : ""} key={revision.id}>
            <span>{revision.id.slice(0, 8)}</span>
            <small>{new Date(revision.createdAt).toLocaleTimeString()}</small>
          </li>
        ))}
      </ol>
    </section>
  );
}

function ArtifactList({ artifacts }: { artifacts: CadArtifact[] }) {
  return (
    <ul className="artifacts">
      {artifacts.map((artifact) => (
        <li key={artifact.id}>
          <a href={artifact.uri} target="_blank" rel="noreferrer">
            {artifact.kind}.{artifact.format}
          </a>
        </li>
      ))}
    </ul>
  );
}

function sessionIdFromPath(): string | null {
  const match = window.location.pathname.match(/^\/sessions\/([^/]+)/);
  return match?.[1] ?? null;
}

function replaceUrl(uiUrl: string): void {
  window.history.replaceState({}, "", toHistoryPath(uiUrl, window.location.href));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
