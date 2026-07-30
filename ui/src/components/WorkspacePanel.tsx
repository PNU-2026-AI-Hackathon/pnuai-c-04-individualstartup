import { Play, Save } from "lucide-react";
import type {
  CadAgentRun,
  CadMesh,
  CadParameter,
  CadRevision,
  CadSessionState
} from "../protocol";
import type { OpenscadRuntimeState } from "../runtime/openscadRuntime";
import { MeshPreview } from "../MeshPreview";
import { AgentWorkspace } from "./AgentWorkspace";
import { Diagnostics, Parameters, Timeline } from "./RevisionPanels";

type WorkspacePanelProps = {
  state: CadSessionState;
  mesh: CadMesh | null;
  runtimeState: OpenscadRuntimeState;
  busy: boolean;
  sessionArchived: boolean;
  source: string;
  sourceDirty: boolean;
  activeRevision?: CadRevision;
  activeRun?: CadAgentRun;
  agentPrompt: string;
  onRenderPreview: () => void | Promise<void>;
  onSaveSource: () => void | Promise<void>;
  onEditSource: (value: string) => void;
  onPromptChange: (value: string) => void;
  onStartRun: () => void | Promise<void>;
  onRetryRun: (run: CadAgentRun) => void | Promise<void>;
  onCancelRun: (runId: string) => void | Promise<void>;
  onUpdateParameter: (parameter: CadParameter, value: CadParameter["value"]) => void | Promise<void>;
  onActivateRevision: (revisionId: string) => void | Promise<void>;
  onRestoreRevision: (revisionId: string) => void | Promise<void>;
};

export function WorkspacePanel({
  state,
  mesh,
  runtimeState,
  busy,
  sessionArchived,
  source,
  sourceDirty,
  activeRevision,
  activeRun,
  agentPrompt,
  onRenderPreview,
  onSaveSource,
  onEditSource,
  onPromptChange,
  onStartRun,
  onRetryRun,
  onCancelRun,
  onUpdateParameter,
  onActivateRevision,
  onRestoreRevision
}: WorkspacePanelProps) {
  return (
    <section className="workspace">
      <div className="preview-pane">
        <div className="pane-toolbar">
          <h2>Preview</h2>
          <span className={`runtime-state runtime-state-${runtimeState}`}>{runtimeState}</span>
          <button onClick={onRenderPreview} disabled={busy || sessionArchived} title="Render preview">
            <Play size={16} /> Render
          </button>
        </div>
        <MeshPreview mesh={mesh} />
        <Diagnostics state={state} />
      </div>

      <div className="editor-pane">
        <div className="pane-toolbar">
          <h2>OpenSCAD Source</h2>
          <button onClick={onSaveSource} disabled={busy || !sourceDirty || sessionArchived} title="Save source revision">
            <Save size={16} /> Save
          </button>
        </div>
        <textarea
          data-testid="source-editor"
          value={source}
          onChange={(event) => onEditSource(event.target.value)}
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
          activeRun={activeRun}
          onPromptChange={onPromptChange}
          onStartRun={onStartRun}
          onRetryRun={(run) => onRetryRun(run)}
          onCancelRun={onCancelRun}
        />
        <Parameters revision={activeRevision} readOnly={sessionArchived} onUpdate={onUpdateParameter} />
        <Timeline
          state={state}
          busy={busy}
          readOnly={sessionArchived}
          sourceDirty={sourceDirty}
          onActivate={onActivateRevision}
          onRestore={onRestoreRevision}
        />
      </aside>
    </section>
  );
}
