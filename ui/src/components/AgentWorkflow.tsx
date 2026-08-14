import type {
  CadAgentRun,
  CadAgentRunEvent,
  CadDfmReport,
  CadValidationBatch,
  CadValidationCheck,
  CadValidationEvaluation,
  CadWorkflowOuterIteration,
  CadWorkflowPendingVlm,
  CadWorkflowState
} from "../protocol";

export interface WorkflowRunView {
  stage: string;
  finalizationStatus: string;
  plan?: CadWorkflowState["plans"][number];
  iterations: CadWorkflowOuterIteration[];
  pendingVlm?: CadWorkflowPendingVlm;
  validationEvaluation?: CadValidationEvaluation;
  validationBatch?: CadValidationBatch;
  validationChecks: CadValidationCheck[];
  latestDfmReport?: CadDfmReport;
  latestFailure?: Record<string, unknown>;
  latestCommand?: string;
  latestNextAction?: string;
}

export interface AgentProgressCommentary {
  key: string;
  text: string;
  sequence: number;
  createdAt?: string;
  streaming: boolean;
}

export function AgentRunProgressDetails({
  run,
  commentary,
  view
}: {
  run: CadAgentRun;
  commentary: AgentProgressCommentary[];
  view: WorkflowRunView;
}) {
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
          <dt>commentary</dt>
          <dd>{commentary.length}</dd>
        </div>
      </div>
      {run.error ? <p className="agent-progress-error">{run.error}</p> : null}
      {commentary.length ? (
        <ol className="agent-progress-commentary-list" data-testid="agent-progress-commentary-list">
          {commentary.map((item) => (
            <li key={item.key}>
              <details className="agent-progress-commentary" data-testid="agent-progress-commentary">
                <summary>
                  <span>Live commentary</span>
                  <small>
                    {item.streaming
                      ? "live"
                      : item.createdAt
                        ? new Date(item.createdAt).toLocaleTimeString()
                        : "recorded"}
                  </small>
                </summary>
                <p>{item.text}</p>
              </details>
            </li>
          ))}
        </ol>
      ) : (
        <p className="agent-progress-empty">
          {isActiveRunStatus(run.status) ? "Waiting for live commentary." : "No commentary recorded."}
        </p>
      )}
    </details>
  );
}

export function WorkflowRunSummary({
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
      {view.validationEvaluation ? (
        <ValidationEvaluationSummary evaluation={view.validationEvaluation} />
      ) : null}
      {view.validationBatch ? (
        <ValidationBatchSummary batch={view.validationBatch} checks={view.validationChecks} />
      ) : null}
      {view.latestDfmReport ? <DfmReportSummary report={view.latestDfmReport} compact={compact} /> : null}
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

export function EventPayloadSummary({ event }: { event: CadAgentRunEvent }) {
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

export function workflowRunView(
  run: CadAgentRun,
  events: CadAgentRunEvent[],
  workflow: CadWorkflowState,
  validationEvaluations: CadValidationEvaluation[],
  validationBatches: CadValidationBatch[],
  validationChecks: CadValidationCheck[]
): WorkflowRunView {
  const plan = workflow.plans.find((item) => item.runId === run.id);
  const iterations = workflow.outerIterations
    .filter((item) => item.runId === run.id)
    .sort((left, right) => left.iteration - right.iteration);
  const pendingVlm = workflow.pendingVlm.find((item) => item.runId === run.id);
  const validationBatch = latestValidationBatch(validationBatches, run.id, run.outputRevisionId);
  const batchChecks = validationBatch
    ? validationChecksForBatch(validationChecks, validationBatch.id)
    : [];
  const validationEvaluation = validationBatch
    ? undefined
    : latestValidationEvaluation(validationEvaluations, run.id, run.outputRevisionId);
  const latestDfmReport = validationBatch
    ? undefined
    : [...iterations].reverse().find((iteration) => iteration.dfmReport)?.dfmReport
      ?? pendingVlm?.dfmReport;
  const batchFailure = validationBatch?.status === "succeeded"
    ? recordField(requiredAggregateReport(validationBatch), "failureReport")
    : undefined;
  const latestFailure = validationBatch
    ? batchFailure
    : [...iterations].reverse().find((iteration) => iteration.failureReport)?.failureReport;
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
    stage: workflowStage(run, events, Boolean(plan), iterations, validationBatch, validationEvaluation, pendingVlm, latestFailure, latestDfmReport),
    finalizationStatus: finalizationStatus(run, events, iterations, validationBatch, validationEvaluation, pendingVlm, latestFailure),
    plan,
    iterations,
    pendingVlm: validationBatch || validationEvaluation ? undefined : pendingVlm,
    validationEvaluation,
    validationBatch,
    validationChecks: batchChecks,
    latestDfmReport,
    latestFailure,
    latestCommand,
    latestNextAction
  };
}

export function isActiveRunStatus(status: CadAgentRun["status"]): boolean {
  return status === "queued" || status === "running" || status === "waiting_for_user";
}

function workflowStage(
  run: CadAgentRun,
  events: CadAgentRunEvent[],
  hasPlan: boolean,
  iterations: CadWorkflowOuterIteration[],
  validationBatch?: CadValidationBatch,
  validationEvaluation?: CadValidationEvaluation,
  pendingVlm?: CadWorkflowPendingVlm,
  latestFailure?: Record<string, unknown>,
  latestDfmReport?: CadDfmReport
): string {
  if (validationBatch) {
    if (validationBatch.status === "queued") return "Validation queued";
    if (validationBatch.status === "running") return "Parallel validation running";
    if (validationBatch.status === "failed") return "Validation operational failure";
    return validationBatchPassed(validationBatch) ? "Validation accepted" : "Validation repair";
  }
  if (validationEvaluation) {
    if (validationEvaluation.status === "queued") return "VLM queued";
    if (validationEvaluation.status === "running") return "VLM evaluating";
    if (validationEvaluation.status === "succeeded" && validationEvaluation.passed === true) return "VLM accepted";
    if (validationEvaluation.status === "succeeded" && validationEvaluation.passed === false) return "VLM repair";
    return "VLM failed";
  }
  if (pendingVlm) return "VLM pending";
  if (iterations.some((iteration) => iteration.passed)) return "VLM accepted";
  if (latestFailure) {
    const reason = failureReason(latestFailure).toLowerCase();
    if (reason.includes("vlm")) return "VLM repair";
    if (reason.includes("dfm") || reason.includes("slic") || reason.includes("prusa")) return "DFM repair";
    return "Structural repair";
  }
  if (latestDfmReport && !latestDfmReport.passed) return "DFM repair";
  if (hasCompletedCommand(events, "cadastrophe-finalize")) return "Finalized";
  if (hasCompletedCommand(events, "cadastrophe-source-apply")) return "Source applied";
  if (hasPlan) return "Plan committed";
  if (isActiveRunStatus(run.status)) return "Planning";
  return "Plan required";
}

function DfmReportSummary({ report, compact }: { report: CadDfmReport; compact: boolean }) {
  return (
    <div className={`workflow-callout ${report.passed ? "workflow-dfm-pass" : "workflow-failure"}`} data-testid="dfm-report-summary">
      <strong>DFM {report.passed ? "passed" : "failed"}</strong>
      <span>{report.checks.length} checks · {report.diagnostics.length} diagnostics</span>
      <code>profile {report.profileHash}</code>
      {report.gcodeArtifactId ? <code>G-code {shortId(report.gcodeArtifactId)}</code> : null}
      {!compact && report.keySettings ? (
        <dl className="dfm-key-settings">
          {Object.entries(report.keySettings).map(([key, value]) => (
            <div key={key}><dt>{key.replaceAll("_", " ")}</dt><dd>{value}</dd></div>
          ))}
        </dl>
      ) : null}
    </div>
  );
}

function finalizationStatus(
  run: CadAgentRun,
  events: CadAgentRunEvent[],
  iterations: CadWorkflowOuterIteration[],
  validationBatch?: CadValidationBatch,
  validationEvaluation?: CadValidationEvaluation,
  pendingVlm?: CadWorkflowPendingVlm,
  latestFailure?: Record<string, unknown>
): string {
  if (validationBatch) {
    if (validationBatch.status === "queued") return "waiting for validation";
    if (validationBatch.status === "running") return "validation running";
    if (validationBatch.status === "failed") return "operational failure";
    return validationBatchPassed(validationBatch) ? "passed" : "rejected";
  }
  if (validationEvaluation) {
    if (validationEvaluation.status === "queued") return "waiting for VLM";
    if (validationEvaluation.status === "running") return "VLM evaluation running";
    if (validationEvaluation.status === "succeeded") {
      return validationEvaluation.passed === true ? "passed" : "failed";
    }
    return "failed";
  }
  if (iterations.some((iteration) => iteration.passed)) return "passed";
  if (pendingVlm) return "waiting for VLM";
  if (latestFailure) return "failed";
  if (hasCompletedCommand(events, "cadastrophe-finalize")) return "structural passed";
  if (run.status === "completed") return "completed";
  if (run.status === "failed" || run.status === "cancelled") return run.status;
  return "not finalized";
}

function ValidationBatchSummary({
  batch,
  checks
}: {
  batch: CadValidationBatch;
  checks: CadValidationCheck[];
}) {
  const failed = batch.status === "failed"
    || (batch.status === "succeeded" && !validationBatchPassed(batch));
  const outcome = batch.status === "succeeded"
    ? validationBatchPassed(batch) ? "passed" : "rejected"
    : batch.status === "failed" ? "operational failure" : batch.status;
  return (
    <div
      className={`workflow-callout ${failed ? "workflow-failure" : "workflow-pending"}`}
      data-testid="validation-batch-summary"
    >
      <strong>Validation batch {outcome}</strong>
      <span>attempt {batch.attempt} · artifact {shortId(batch.artifactId)}</span>
      <ol>
        {checks.map((check) => (
          <li data-testid={`validation-check-${check.kind}`} key={check.id}>
            <span>{validationCheckLabel(check.kind)}: {validationCheckOutcome(check)}</span>
            {check.error ? <small>{check.error}</small> : null}
          </li>
        ))}
      </ol>
    </div>
  );
}

export function latestValidationBatch(
  batches: CadValidationBatch[],
  runId: string,
  outputRevisionId?: string
): CadValidationBatch | undefined {
  if (!outputRevisionId) return undefined;
  return batches
    .filter((batch) => batch.runId === runId && batch.revisionId === outputRevisionId)
    .sort((left, right) =>
      left.attempt - right.attempt
      || left.createdAt.localeCompare(right.createdAt)
      || left.id.localeCompare(right.id)
    )
    .at(-1);
}

export function validationChecksForBatch(
  checks: CadValidationCheck[],
  batchId: string
): CadValidationCheck[] {
  const selected = checks.filter((check) => check.batchId === batchId);
  const kinds = new Set(selected.map((check) => check.kind));
  if (selected.length !== 3 || kinds.size !== 3
    || !kinds.has("structural") || !kinds.has("dfm") || !kinds.has("vlm")) {
    throw new Error(`Validation batch ${batchId} must have exactly one structural, DFM, and VLM check.`);
  }
  const order = { structural: 0, dfm: 1, vlm: 2 } as const;
  return selected.sort((left, right) => order[left.kind] - order[right.kind]);
}

export function validationBatchPassed(batch: CadValidationBatch): boolean {
  if (batch.status !== "succeeded") return false;
  const passed = requiredAggregateReport(batch).passed;
  if (typeof passed !== "boolean") {
    throw new Error(`Succeeded validation batch ${batch.id} aggregate report is missing boolean passed.`);
  }
  return passed;
}

function requiredAggregateReport(batch: CadValidationBatch): Record<string, unknown> {
  if (!batch.aggregateReport) {
    throw new Error(`Succeeded validation batch ${batch.id} is missing aggregate report.`);
  }
  return batch.aggregateReport;
}

function validationCheckLabel(kind: CadValidationCheck["kind"]): string {
  if (kind === "dfm") return "DFM";
  if (kind === "vlm") return "VLM";
  return "Structural";
}

function validationCheckOutcome(check: CadValidationCheck): string {
  if (check.status === "succeeded") return check.passed ? "passed" : "rejected";
  if (check.status === "failed") return "operational failure";
  return check.status;
}

function ValidationEvaluationSummary({ evaluation }: { evaluation: CadValidationEvaluation }) {
  const outcome = evaluation.status === "succeeded"
    ? evaluation.passed === true ? "passed" : "failed"
    : evaluation.status;
  return (
    <div
      className={`workflow-callout ${outcome === "failed" ? "workflow-failure" : "workflow-pending"}`}
      data-testid="validation-evaluation-summary"
    >
      <strong>VLM evaluation {outcome}</strong>
      <span>attempt {evaluation.attempt} · artifact {shortId(evaluation.artifactId)} · threshold {evaluation.passThreshold}</span>
      {evaluation.score !== undefined ? <code>score {evaluation.score}</code> : null}
      {evaluation.error ? <span>{evaluation.error}</span> : null}
    </div>
  );
}

export function latestValidationEvaluation(
  evaluations: CadValidationEvaluation[],
  runId: string,
  outputRevisionId?: string
): CadValidationEvaluation | undefined {
  const runEvaluations = evaluations.filter((evaluation) => evaluation.runId === runId);
  const revisionEvaluations = outputRevisionId
    ? runEvaluations.filter((evaluation) => evaluation.revisionId === outputRevisionId)
    : runEvaluations;
  return revisionEvaluations
    .sort((left, right) =>
      left.createdAt.localeCompare(right.createdAt)
      || left.attempt - right.attempt
      || left.id.localeCompare(right.id)
    )
    .at(-1);
}

function hasCompletedCommand(events: CadAgentRunEvent[], command: string): boolean {
  return events.some((event) => {
    if (event.type !== "agent.tool.completed") return false;
    const payloadCommand = stringField(event.payload, "command") ?? stringField(event.payload, "tool");
    return payloadCommand?.startsWith(command) ?? false;
  });
}

export function failureTitle(report: Record<string, unknown>): string {
  const reason = failureReason(report).toLowerCase();
  if (reason.includes("vlm")) return "VLM failure report";
  if (reason.includes("dfm") || reason.includes("slic") || reason.includes("prusa")) return "DFM failure report";
  return "Structural failure report";
}

function failureReason(report: Record<string, unknown>): string {
  return stringField(report, "reason") ?? stringField(report, "code") ?? "";
}

export function failureSummary(report: Record<string, unknown>): string {
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

export function shortId(value?: string): string {
  return value ? value.slice(0, 8) : "-";
}

export function formatPayload(payload: Record<string, unknown>): string {
  return JSON.stringify(payload, null, 2);
}
