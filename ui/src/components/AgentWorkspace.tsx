import { MessageSquarePlus, RefreshCcw, ScrollText, Send, X } from "lucide-react";
import type {
  CadAgentRun,
  CadAgentRunEvent,
  CadAgentThread,
  CadConversationMessage,
  CadWorkflowState
} from "../protocol";
import type { CadAgentStreamingItem } from "../runtime/agentStream";
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
  threads: CadAgentThread[];
  events: CadAgentRunEvent[];
  workflow: CadWorkflowState;
  prompt: string;
  busy: boolean;
  readOnly: boolean;
  activeRun?: CadAgentRun;
  streams?: CadAgentStreamingItem[];
  onPromptChange: (value: string) => void;
  onStartRun: () => void;
  onStartNewConversation: () => void;
  onRetryRun: (run: CadAgentRun) => void;
  onCancelRun: (runId: string) => void;
  onOpenFullHistory: () => void;
}) {
  const latestRun = props.runs.at(-1);
  const conversation = buildAgentConversation(props.conversation);
  const streams = splitAgentStreams(props.streams ?? []);
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
          <div className="agent-debug-popover">
            <AgentDiagnostics threads={props.threads} runs={props.runs} />
            <button onClick={props.onOpenFullHistory} title="Open full session history">
              <ScrollText size={15} /> Full history
            </button>
          </div>
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
        {streams.finalAnswers.map((item) => (
          <li
            className="conversation-item conversation-assistant conversation-streaming"
            data-testid="streaming-final-answer"
            key={`stream-${item.runId}-${item.itemId}`}
          >
            <span>assistant · streaming</span>
            <p>{item.text}</p>
          </li>
        ))}
      </ol>
      {streams.commentary.length ? (
        <details className="agent-stream-commentary" open data-testid="streaming-commentary">
          <summary>Live commentary</summary>
          {streams.commentary.map((item) => (
            <p key={`stream-${item.runId}-${item.itemId}`}>{item.text}</p>
          ))}
        </details>
      ) : null}
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
        <button
          data-testid="start-new-agent-conversation"
          onClick={props.onStartNewConversation}
          disabled={props.busy || props.readOnly || Boolean(props.activeRun)}
          title="Archive this Codex thread and start a new conversation"
        >
          <MessageSquarePlus size={16} /> New conversation
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

export function agentDiagnosticRows(threads: CadAgentThread[], runs: CadAgentRun[]) {
  const threadRows = threads.map((thread) => ({
    key: `thread-${thread.id}`,
    label: `${thread.externalAgent} thread`,
    value: `mapping ${thread.id} · external ${thread.externalThreadId} · ${thread.status} · generation ${thread.connectionGeneration ?? "—"}`
  }));
  const runRows = runs.map((run) => ({
    key: `run-${run.id}`,
    label: `run ${run.id}`,
    value: [
      run.status,
      `recovery ${run.recoveryStatus}`,
      `thread ${run.externalThreadId ?? "—"}`,
      `turn ${run.externalTurnId ?? "—"}`
    ].join(" · ")
  }));
  return [...threadRows, ...runRows];
}

function AgentDiagnostics({ threads, runs }: { threads: CadAgentThread[]; runs: CadAgentRun[] }) {
  const rows = agentDiagnosticRows(threads, runs);
  return (
    <section className="agent-diagnostics" data-testid="agent-diagnostics">
      <strong>Agent identifiers</strong>
      <small>{rows.length ? `${threads.length} threads · ${runs.length} runs` : "No agent threads or runs"}</small>
      {rows.map((row) => (
        <div key={row.key}>
          <span>{row.label}</span>
          <code>{row.value}</code>
        </div>
      ))}
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

export function buildAgentConversation(conversation: CadConversationMessage[]) {
  return conversation
    .filter((message) =>
      message.role === "user" ||
      (message.role === "assistant" && message.phase !== "commentary")
    )
    .sort((left, right) => left.createdAt.localeCompare(right.createdAt));
}

export function splitAgentStreams(streams: CadAgentStreamingItem[]) {
  return {
    commentary: streams.filter((item) => item.phase === "commentary"),
    finalAnswers: streams.filter((item) => item.phase === "final_answer")
  };
}
