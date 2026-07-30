import { RefreshCcw, ScrollText, Send, X } from "lucide-react";
import type {
  CadAgentRun,
  CadAgentRunEvent,
  CadConversationMessage,
  CadWorkflowState
} from "../protocol";
import {
  AgentRunProgressDetails,
  WorkflowRunSummary,
  failureSummary,
  failureTitle,
  formatPayload,
  workflowRunView
} from "./AgentWorkflow";

export function AgentWorkspace(props: {
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
  onOpenFullHistory: () => void;
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
        <details className="agent-debug-menu">
          <summary>Debug</summary>
          <button onClick={props.onOpenFullHistory} title="Open full session history">
            <ScrollText size={15} /> Full history
          </button>
        </details>
      </div>
      {props.activeRun?.activeStep ? (
        <div className="active-step" data-testid="active-step">
          {props.activeRun.activeStep.replaceAll("_", " ")}
        </div>
      ) : null}
      {latestRun && latestWorkflow ? (
        <WorkflowRunSummary run={latestRun} view={latestWorkflow} compact />
      ) : null}
      {latestRun && latestWorkflow?.latestFailure ? (
        <AgentFailureAction
          run={latestRun}
          failure={latestWorkflow.latestFailure}
          nextAction={latestWorkflow.latestNextAction}
          busy={props.busy}
          readOnly={props.readOnly}
          activeRun={props.activeRun}
          onRetryRun={props.onRetryRun}
        />
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
      {(latestRun?.status === "failed" || latestRun?.status === "cancelled") && !latestWorkflow?.latestFailure ? (
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

function AgentFailureAction({
  run,
  failure,
  nextAction,
  busy,
  readOnly,
  activeRun,
  onRetryRun
}: {
  run: CadAgentRun;
  failure: Record<string, unknown>;
  nextAction?: string;
  busy: boolean;
  readOnly: boolean;
  activeRun?: CadAgentRun;
  onRetryRun: (run: CadAgentRun) => void;
}) {
  return (
    <section className="agent-failure-action" data-testid="agent-failure-action">
      <div>
        <strong>{failureTitle(failure)}</strong>
        <p>{failureSummary(failure)}</p>
        <small>Next action: {nextAction ? nextAction.replaceAll("_", " ") : "revise source and rerun"}</small>
      </div>
      {run.status === "failed" || run.status === "cancelled" ? (
        <button
          onClick={() => onRetryRun(run)}
          disabled={busy || readOnly || Boolean(activeRun)}
          title="Retry the failed agent run"
        >
          <RefreshCcw size={16} /> Retry
        </button>
      ) : null}
      <details className="advanced-disclosure">
        <summary>Advanced failure payload</summary>
        <code>{formatPayload(failure)}</code>
      </details>
    </section>
  );
}

function buildAgentConversation(conversation: CadConversationMessage[]) {
  return conversation
    .filter((message) => message.role === "user" || message.role === "assistant")
    .sort((left, right) => left.createdAt.localeCompare(right.createdAt));
}
