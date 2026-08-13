import type {
  CadAgentRun,
  CadAgentRunEvent,
  CadConversationMessage,
  CadValidationEvaluation,
  CadWorkflowState
} from "../protocol";
import {
  EventPayloadSummary,
  WorkflowRunSummary,
  formatPayload,
  isActiveRunStatus,
  shortId,
  workflowRunView
} from "./AgentWorkflow";

export function SessionLogs({
  conversation,
  runs,
  events,
  workflow,
  validationEvaluations
}: {
  conversation: CadConversationMessage[];
  runs: CadAgentRun[];
  events: CadAgentRunEvent[];
  workflow: CadWorkflowState;
  validationEvaluations: CadValidationEvaluation[];
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
      <RunLogViewer
        runs={runs}
        events={events}
        conversation={conversation}
        workflow={workflow}
        validationEvaluations={validationEvaluations}
      />
    </section>
  );
}

function RunLogViewer({
  runs,
  events,
  conversation,
  workflow,
  validationEvaluations
}: {
  runs: CadAgentRun[];
  events: CadAgentRunEvent[];
  conversation: CadConversationMessage[];
  workflow: CadWorkflowState;
  validationEvaluations: CadValidationEvaluation[];
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
        const workflowView = workflowRunView(run, runEvents, workflow, validationEvaluations);
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
