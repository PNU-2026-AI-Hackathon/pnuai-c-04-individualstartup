import {
  Component,
  useEffect,
  useMemo,
  useState,
  type ChangeEvent,
  type ErrorInfo,
  type FocusEvent,
  type ReactNode
} from "react";
import { Check, Download, Play, Save } from "lucide-react";
import type {
  CadAgentRun,
  CadDiagnostic,
  CadDiagnostics,
  CadMesh,
  CadParameter,
  CadRevision,
  CadSessionState
} from "../protocol";
import type { OpenscadRuntimeState } from "../runtime/openscadRuntime";
import { MeshPreview } from "../MeshPreview";
import { AgentWorkspace } from "./AgentWorkspace";
import { workflowRunView } from "./AgentWorkflow";
import { Parameters, Timeline } from "./RevisionPanels";

type WorkspacePanelProps = {
  state: CadSessionState;
  mesh: CadMesh | null;
  runtimeState: OpenscadRuntimeState;
  busy: boolean;
  sessionArchived: boolean;
  source: string;
  starterSource?: string;
  sourceDirty: boolean;
  showStarterOverlay: boolean;
  activeRevision?: CadRevision;
  activeRun?: CadAgentRun;
  agentPrompt: string;
  onRenderPreview: () => void | Promise<void>;
  onSaveSource: () => void | Promise<void>;
  onEditSource: (value: string) => void;
  onDismissStarterOverlay: () => void;
  onPromptChange: (value: string) => void;
  onStartRun: () => void | Promise<void>;
  onRetryRun: (run: CadAgentRun) => void | Promise<void>;
  onCancelRun: (runId: string) => void | Promise<void>;
  onUpdateParameter: (parameter: CadParameter, value: CadParameter["value"]) => void | Promise<void>;
  onActivateRevision: (revisionId: string) => void | Promise<void>;
  onRestoreRevision: (revisionId: string) => void | Promise<void>;
  onExport: (format: "stl" | "metadata", revisionId?: string) => void | Promise<void>;
  onOpenFullHistory: () => void;
};

export function WorkspacePanel({
  state,
  mesh,
  runtimeState,
  busy,
  sessionArchived,
  source,
  starterSource,
  sourceDirty,
  showStarterOverlay,
  activeRevision,
  activeRun,
  agentPrompt,
  onRenderPreview,
  onSaveSource,
  onEditSource,
  onDismissStarterOverlay,
  onPromptChange,
  onStartRun,
  onRetryRun,
  onCancelRun,
  onUpdateParameter,
  onActivateRevision,
  onRestoreRevision,
  onExport,
  onOpenFullHistory
}: WorkspacePanelProps) {
  const [inspectorTab, setInspectorTab] = useState<InspectorTab>(() =>
    preferredInspectorTab(state, activeRun)
  );
  const [exportSelectorOpen, setExportSelectorOpen] = useState(false);
  const [selectedExportFormat, setSelectedExportFormat] = useState<ExportFormat>("stl");

  useEffect(() => {
    setInspectorTab(preferredInspectorTab(state, activeRun));
  }, [activeRun?.id, activeRun?.status, state.activeRevision?.parameters.length]);

  return (
    <section className="workspace">
      <main className="modeling-area">
        <WorkflowProgressStrip state={state} activeRun={activeRun} runtimeState={runtimeState} />
        <div className="preview-pane">
          <div className="pane-toolbar preview-toolbar">
            <h2>Preview</h2>
            <div className="toolbar-actions">
              <span className={`runtime-state runtime-state-${runtimeState}`}>
                {runtimeStateLabel(runtimeState)}
              </span>
              <button onClick={onRenderPreview} disabled={busy || sessionArchived} title="Render preview">
                <Play size={16} /> Render
              </button>
              <div className="export-toolbar-control">
                <button
                  onClick={() => setExportSelectorOpen((open) => !open)}
                  disabled={busy || sessionArchived || !activeRevision?.id}
                  title="Export current model"
                  aria-expanded={exportSelectorOpen}
                  aria-haspopup="dialog"
                >
                  <Download size={16} /> Export
                </button>
                {exportSelectorOpen ? (
                  <ExportFormatSelector
                    selectedFormat={selectedExportFormat}
                    busy={busy}
                    readOnly={sessionArchived}
                    revisionId={activeRevision?.id}
                    onSelectFormat={setSelectedExportFormat}
                    onExport={async () => {
                      await onExport(selectedExportFormat, activeRevision?.id);
                      setExportSelectorOpen(false);
                    }}
                  />
                ) : null}
              </div>
            </div>
          </div>
          <MeshPreview mesh={mesh} />
        </div>

        <div className="editor-pane">
          <div className="pane-toolbar">
            <h2>OpenSCAD Source</h2>
            <button
              onClick={() => {
                onDismissStarterOverlay();
                onSaveSource();
              }}
              disabled={busy || !sourceDirty || sessionArchived}
              title="Save source revision"
            >
              <Save size={16} /> Save revision
            </button>
          </div>
          <SourceEditorErrorBoundary sourceIdentity={sourceEditorIdentity(state, activeRevision)}>
            <SourceEditor
              sessionId={state.session.id}
              revisionId={activeRevision?.id}
              source={source}
              starterSource={starterSource}
              showStarterOverlay={showStarterOverlay}
              readOnly={sessionArchived}
              onDismissStarterOverlay={onDismissStarterOverlay}
              onEditSource={onEditSource}
            />
          </SourceEditorErrorBoundary>
          <SourceDiagnostics diagnostics={activeRevision?.diagnostics} source={source} />
        </div>
      </main>

      <aside className="workspace-inspector" aria-label="Workspace inspector">
        <div className="inspector-tabs" role="tablist" aria-label="Inspector sections">
          {INSPECTOR_TABS.map((tab) => (
            <button
              className={inspectorTab === tab.id ? "active" : ""}
              key={tab.id}
              onClick={() => setInspectorTab(tab.id)}
              role="tab"
              aria-selected={inspectorTab === tab.id}
            >
              {tab.label}
            </button>
          ))}
        </div>
        <div className="inspector-body">
          {inspectorTab === "agent" ? (
            <AgentWorkspace
              conversation={state.conversation}
              runs={state.agentRuns}
              events={state.agentRunEvents}
              workflow={state.workflow}
              prompt={agentPrompt}
              busy={busy}
              readOnly={sessionArchived}
              activeRun={activeRun}
              onPromptChange={onPromptChange}
              onStartRun={onStartRun}
              onRetryRun={(run) => onRetryRun(run)}
              onCancelRun={onCancelRun}
              onOpenFullHistory={onOpenFullHistory}
            />
          ) : null}
          {inspectorTab === "parameters" ? (
            <Parameters revision={activeRevision} readOnly={sessionArchived} onUpdate={onUpdateParameter} />
          ) : null}
          {inspectorTab === "revisions" ? (
            <Timeline
              state={state}
              busy={busy}
              readOnly={sessionArchived}
              sourceDirty={sourceDirty}
              onActivate={onActivateRevision}
              onRestore={onRestoreRevision}
            />
          ) : null}
        </div>
      </aside>
    </section>
  );
}

type InspectorTab = "agent" | "parameters" | "revisions";
type ExportFormat = "stl";

const INSPECTOR_TABS: Array<{ id: InspectorTab; label: string }> = [
  { id: "agent", label: "Agent" },
  { id: "parameters", label: "Parameters" },
  { id: "revisions", label: "Revisions" }
];

type SourceEditorErrorBoundaryProps = {
  children: ReactNode;
  sourceIdentity: string;
};

type SourceEditorErrorBoundaryState = {
  error: string | null;
};

class SourceEditorErrorBoundary extends Component<
  SourceEditorErrorBoundaryProps,
  SourceEditorErrorBoundaryState
> {
  state: SourceEditorErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: unknown): SourceEditorErrorBoundaryState {
    return { error: editorErrorMessage(error) };
  }

  componentDidCatch(error: unknown, info: ErrorInfo) {
    console.error("[cadastrophe] Source editor render failed", {
      error,
      componentStack: info.componentStack
    });
  }

  componentDidUpdate(previousProps: SourceEditorErrorBoundaryProps) {
    if (previousProps.sourceIdentity !== this.props.sourceIdentity && this.state.error) {
      this.setState({ error: null });
    }
  }

  render() {
    if (this.state.error) return <SourceEditorFallback message={this.state.error} />;
    return this.props.children;
  }
}

function SourceEditor({
  sessionId,
  revisionId,
  source,
  starterSource,
  showStarterOverlay,
  readOnly,
  onDismissStarterOverlay,
  onEditSource
}: {
  sessionId: string;
  revisionId?: string;
  source: string;
  starterSource?: string;
  showStarterOverlay: boolean;
  readOnly: boolean;
  onDismissStarterOverlay: () => void;
  onEditSource: (value: string) => void;
}) {
  const [eventError, setEventError] = useState<string | null>(null);

  useEffect(() => {
    setEventError(null);
  }, [sessionId, revisionId]);

  const handleEditorError = (phase: "focus" | "change", error: unknown) => {
    console.error("[cadastrophe] Source editor event failed", {
      phase,
      sessionId,
      revisionId,
      error
    });
    setEventError(editorErrorMessage(error));
  };

  const handleFocus = (_event: FocusEvent<HTMLTextAreaElement>) => {
    try {
      onDismissStarterOverlay();
    } catch (error) {
      handleEditorError("focus", error);
    }
  };

  const handleChange = (event: ChangeEvent<HTMLTextAreaElement>) => {
    const nextSource = event.target.value;
    try {
      onDismissStarterOverlay();
      onEditSource(nextSource);
    } catch (error) {
      handleEditorError("change", error);
    }
  };

  if (eventError) return <SourceEditorFallback message={eventError} />;

  return (
    <div className={showStarterOverlay ? "source-editor-wrap starter-visible" : "source-editor-wrap"}>
      <textarea
        data-testid="source-editor"
        value={source}
        onFocus={handleFocus}
        onChange={handleChange}
        readOnly={readOnly}
        spellCheck={false}
      />
      {showStarterOverlay ? (
        <pre className="source-starter-overlay" aria-hidden="true">{starterSource ?? source}</pre>
      ) : null}
    </div>
  );
}

function SourceEditorFallback({ message }: { message: string }) {
  return (
    <div className="source-editor-fallback" role="alert">
      <strong>Source editor unavailable</strong>
      <span>{message}</span>
    </div>
  );
}

function sourceEditorIdentity(state: CadSessionState, activeRevision?: CadRevision): string {
  return `${state.session.id}:${activeRevision?.id ?? state.session.activeRevisionId ?? "none"}`;
}

function editorErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function preferredInspectorTab(state: CadSessionState, activeRun?: CadAgentRun): InspectorTab {
  if (activeRun) return "agent";
  if (state.activeRevision?.parameters.length) return "parameters";
  return "agent";
}

function SourceDiagnostics({
  diagnostics,
  source
}: {
  diagnostics?: CadDiagnostics;
  source: string;
}) {
  const items = diagnostics?.items ?? [];
  const visibleItems = items.filter((item) => item.severity === "error" || item.severity === "warning");
  const errorCount = items.filter((item) => item.severity === "error").length;
  const warningCount = items.filter((item) => item.severity === "warning").length;
  const hasFailure = diagnostics?.ok === false;
  const summary = diagnostics
    ? hasFailure
      ? `${errorCount} error${errorCount === 1 ? "" : "s"}, ${warningCount} warning${warningCount === 1 ? "" : "s"}`
      : `Render passed in ${diagnostics.elapsedMs}ms`
    : "Render has not produced diagnostics yet";

  return (
    <details
      className={hasFailure ? "source-diagnostics source-diagnostics-failed" : "source-diagnostics"}
      data-testid="source-diagnostics"
      open={hasFailure}
    >
      <summary>
        <span>Source diagnostics</span>
        <small>{summary}</small>
      </summary>
      {visibleItems.length ? (
        <ol className="source-diagnostic-list">
          {visibleItems.map((item, index) => (
            <li className={`diagnostic-${item.severity}`} key={`${item.message}-${index}`}>
              <DiagnosticLocation item={item} source={source} />
              <p>{item.message}</p>
            </li>
          ))}
        </ol>
      ) : (
        <p className="source-diagnostic-empty">
          {diagnostics?.ok ? "No source errors or warnings." : "No actionable source diagnostics were reported."}
        </p>
      )}
      {items.length ? (
        <details className="advanced-disclosure">
          <summary>Advanced runtime output</summary>
          <ol>
            {items.map((item, index) => (
              <li key={`${item.severity}-${item.message}-${index}`}>
                <code>{formatDiagnosticLine(item)}</code>
              </li>
            ))}
          </ol>
        </details>
      ) : null}
    </details>
  );
}

function DiagnosticLocation({ item, source }: { item: CadDiagnostic; source: string }) {
  const context = sourceLine(source, item.line);
  if (!item.line && !item.column && !context) {
    return <strong>{item.severity}</strong>;
  }
  return (
    <div className="diagnostic-location">
      <strong>{item.severity}</strong>
      {item.line ? <span>line {item.line}</span> : null}
      {item.column ? <span>col {item.column}</span> : null}
      {context ? <code>{context}</code> : null}
    </div>
  );
}

function sourceLine(source: string, line?: number): string | undefined {
  if (!line || line < 1) return undefined;
  return source.split(/\r?\n/)[line - 1]?.trim() || undefined;
}

function formatDiagnosticLine(item: CadDiagnostic): string {
  const location = item.line
    ? ` line ${item.line}${item.column ? `:${item.column}` : ""}`
    : "";
  return `${item.severity}${location}: ${item.message}`;
}

function WorkflowProgressStrip({
  state,
  activeRun,
  runtimeState
}: {
  state: CadSessionState;
  activeRun?: CadAgentRun;
  runtimeState: OpenscadRuntimeState;
}) {
  const latestRun = state.agentRuns.at(-1);
  const events = useMemo(
    () => latestRun
      ? state.agentRunEvents
          .filter((event) => event.runId === latestRun.id)
          .sort((left, right) => left.sequence - right.sequence)
      : [],
    [latestRun?.id, state.agentRunEvents]
  );
  const latestWorkflow = latestRun ? workflowRunView(latestRun, events, state.workflow) : undefined;
  const steps = workflowSteps(state, latestWorkflow?.latestFailure, Boolean(activeRun), runtimeState);

  return (
    <section className="workflow-progress-strip" aria-label="Workflow progress">
      {steps.map((step, index) => (
        <div className={`workflow-step workflow-step-${step.state}`} key={step.label}>
          <span>{index + 1}</span>
          <strong>{step.label}</strong>
          <small>{step.state}</small>
        </div>
      ))}
    </section>
  );
}

function workflowSteps(
  state: CadSessionState,
  latestFailure: Record<string, unknown> | undefined,
  hasActiveRun: boolean,
  runtimeState: OpenscadRuntimeState
): Array<{ label: string; state: "pending" | "active" | "pass" | "fail" }> {
  const hasPlan = state.workflow.plans.length > 0;
  const isSourceFreeSession =
    !state.activeRevision &&
    !state.session.activeRevisionId &&
    state.session.revisions.length === 0;
  const hasCanonicalPreviewRuntime = !isSourceFreeSession && runtimeState === "completed";
  const hasCanonicalPreviewFailure =
    !isSourceFreeSession && (runtimeState === "failed" || runtimeState === "canceled");
  const hasPreview =
    Boolean(state.activeRevision?.artifacts.some((artifact) => artifact.kind === "preview-mesh")) ||
    hasCanonicalPreviewRuntime;
  const latestIteration = state.workflow.outerIterations.at(-1);
  const failureReason = latestFailure ? stringField(latestFailure, "reason") ?? stringField(latestFailure, "code") ?? "" : "";
  const structuralFailed = Boolean(latestFailure && !failureReason.toLowerCase().includes("vlm"));
  const vlmFailed = Boolean(latestFailure && failureReason.toLowerCase().includes("vlm"));
  const structuralPassed = Boolean(latestIteration && !structuralFailed) || state.workflow.pendingVlm.length > 0;
  const vlmPassed = state.workflow.outerIterations.some((iteration) => iteration.passed);

  return [
    { label: "Plan", state: hasPlan ? "pass" : hasActiveRun ? "active" : "pending" },
    {
      label: "Preview",
      state: hasCanonicalPreviewFailure
        ? "fail"
        : hasPreview
          ? "pass"
          : hasPlan
            ? "active"
            : "pending"
    },
    {
      label: "Structural",
      state: structuralFailed ? "fail" : structuralPassed ? "pass" : hasPreview ? "active" : "pending"
    },
    {
      label: "VLM",
      state: vlmFailed ? "fail" : vlmPassed ? "pass" : state.workflow.pendingVlm.length ? "active" : "pending"
    },
    { label: "Complete", state: vlmPassed ? "pass" : vlmFailed || structuralFailed ? "fail" : "pending" }
  ];
}

function stringField(value: Record<string, unknown>, key: string): string | undefined {
  const field = value[key];
  return typeof field === "string" && field.trim() ? field : undefined;
}

function runtimeStateLabel(state: OpenscadRuntimeState): string {
  if (state === "idle") return "Preview not rendered";
  if (state === "completed") return "Preview ready";
  if (state === "failed") return "Render failed";
  if (state === "canceled") return "Render canceled";
  if (state === "initializing") return "Initializing";
  return "Rendering";
}

function ExportFormatSelector({
  selectedFormat,
  busy,
  readOnly,
  revisionId,
  onSelectFormat,
  onExport
}: {
  selectedFormat: ExportFormat;
  busy: boolean;
  readOnly: boolean;
  revisionId?: string;
  onSelectFormat: (format: ExportFormat) => void;
  onExport: () => void | Promise<void>;
}) {
  return (
    <div className="export-format-popover" role="dialog" aria-label="Export format">
      <div className="export-format-options" role="radiogroup" aria-label="Format">
        <button
          className={selectedFormat === "stl" ? "active" : ""}
          onClick={() => onSelectFormat("stl")}
          role="radio"
          aria-checked={selectedFormat === "stl"}
          type="button"
        >
          {selectedFormat === "stl" ? <Check size={14} /> : null}
          STL
        </button>
        <button disabled title="STEP export is not available yet" type="button">
          STEP
        </button>
        <button disabled title="3MF export is not available yet" type="button">
          3MF
        </button>
      </div>
      <button
        className="export-confirm"
        onClick={onExport}
        disabled={busy || readOnly || !revisionId}
        title="Export selected format"
        type="button"
      >
        <Download size={16} /> Export
      </button>
    </div>
  );
}

function shortId(value?: string): string {
  return value ? value.slice(0, 8) : "-";
}
