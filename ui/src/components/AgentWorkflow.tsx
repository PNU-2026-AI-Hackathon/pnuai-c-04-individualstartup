import type {
  CadAgentRun,
  CadAgentRunEvent,
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
  latestFailure?: Record<string, unknown>;
  latestCommand?: string;
  latestNextAction?: string;
}

export function AgentRunProgressDetails({
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

export function isActiveRunStatus(status: CadAgentRun["status"]): boolean {
  return status === "queued" || status === "running" || status === "waiting_for_user";
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

export function failureTitle(report: Record<string, unknown>): string {
  const reason = failureReason(report);
  return reason.includes("vlm") ? "VLM failure report" : "Structural failure report";
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
