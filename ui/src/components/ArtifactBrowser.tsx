import { useState } from "react";
import { Download, FolderOpen, RefreshCcw, Search, Trash2 } from "lucide-react";
import type { CadArtifact, CadRevisionSummary, VerifyArtifactFilesResult } from "../protocol";

export function ArtifactBrowser({
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
        <h2>Export history</h2>
        <div className="button-row compact">
          <button onClick={() => onExport("stl")} disabled={busy || readOnly} title="Re-export active revision STL">
            <Download size={16} /> Re-export STL
          </button>
          <button onClick={() => onExport("metadata")} disabled={busy || readOnly} title="Export active revision metadata">
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

function artifactStatus(
  artifact: CadArtifact,
  integrityResult: VerifyArtifactFilesResult | null
): "available" | "deleted" | "missing" | "integrity" {
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

function shortId(value?: string): string {
  return value ? value.slice(0, 8) : "-";
}

function formatBytes(value?: number): string {
  if (typeof value !== "number") return "-";
  if (value < 1024) return `${value} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KB`;
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

function formatPayload(payload: Record<string, unknown>): string {
  return JSON.stringify(payload, null, 2);
}
