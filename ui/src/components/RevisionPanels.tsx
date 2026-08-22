import { useState } from "react";
import { GitCompare, RotateCcw, SquareMousePointer } from "lucide-react";
import type { CadParameter, CadRevisionSummary, CadSessionState } from "../protocol";

export function Parameters(props: {
  revision: CadSessionState["activeRevision"];
  readOnly: boolean;
  onUpdate: (parameter: CadParameter, value: CadParameter["value"]) => void;
}) {
  const parameters = props.revision?.parameters ?? [];
  return (
    <section className="panel parameters-panel">
      <h2>Parameters</h2>
      {parameters.map((parameter) => (
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
      {parameters.length === 0 ? (
        <p className="panel-empty">No parameters are available for this revision.</p>
      ) : null}
    </section>
  );
}

export function Diagnostics({ state }: { state: CadSessionState }) {
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

export function Timeline({
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
      {state.session.revisions.length === 0 ? (
        <p className="panel-empty">No revisions yet.</p>
      ) : null}
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
