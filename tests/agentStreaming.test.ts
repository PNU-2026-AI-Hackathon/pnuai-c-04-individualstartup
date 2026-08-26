import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import {
  AgentWorkspace,
  agentDiagnosticRows,
  buildAgentConversation,
  buildRunCommentary,
  splitAgentStreams
} from "../ui/src/components/AgentWorkspace";
import { validationDecisionView } from "../ui/src/components/AgentWorkflow";
import type {
  CadAgentMessagePhase,
  CadAgentStreamEvent,
  CadConversationMessage,
  CadSessionState
} from "../ui/src/protocol";
import {
  createAgentStreamState,
  reconcileAgentStreamSnapshot,
  reduceAgentStreamEvent,
  streamingItems
} from "../ui/src/runtime/agentStream";

test("stream reducer combines deltas per run and item while ignoring duplicate or reversed sequence", () => {
  let state = createAgentStreamState("session-a");
  state = reduceAgentStreamEvent(state, streamEvent({ delta: "Hello", sequence: 10 }));
  state = reduceAgentStreamEvent(state, streamEvent({ delta: " world", sequence: 12 }));
  const current = state;
  state = reduceAgentStreamEvent(state, streamEvent({ delta: " duplicate", sequence: 12 }));
  state = reduceAgentStreamEvent(state, streamEvent({ delta: " reversed", sequence: 11 }));

  assert.equal(state, current);
  assert.equal(streamingItems(state)[0]?.text, "Hello world");

  state = reduceAgentStreamEvent(state, streamEvent({ itemId: "item-2", delta: "Separate", sequence: 13 }));
  state = reduceAgentStreamEvent(state, streamEvent({ runId: "run-2", delta: "Other run", sequence: 14 }));
  assert.deepEqual(streamingItems(state).map((item) => item.text), ["Hello world", "Separate", "Other run"]);
});

test("session transition clears ephemeral state and isolates a late event from the previous session", () => {
  let state = createAgentStreamState("session-a");
  state = reduceAgentStreamEvent(state, streamEvent({ delta: "session A", sequence: 1 }));
  state = reconcileAgentStreamSnapshot(state, snapshot("session-b"));
  assert.equal(state.sessionId, "session-b");
  assert.deepEqual(streamingItems(state), []);

  const afterLateEvent = reduceAgentStreamEvent(
    state,
    streamEvent({ sessionId: "session-a", delta: "late", sequence: 2 })
  );
  assert.equal(afterLateEvent, state);
  assert.deepEqual(streamingItems(afterLateEvent), []);
});

test("completed event removes the ephemeral item and prevents delayed delta recreation", () => {
  let state = createAgentStreamState("session-a");
  state = reduceAgentStreamEvent(state, streamEvent({ delta: "draft", sequence: 1 }));
  state = reduceAgentStreamEvent(state, streamEvent({ delta: "", sequence: 2, completed: true }));
  assert.deepEqual(streamingItems(state), []);

  state = reduceAgentStreamEvent(state, streamEvent({ delta: "late", sequence: 3 }));
  assert.deepEqual(streamingItems(state), []);
});

test("authoritative durable snapshot removes its matching item without duplication", () => {
  let state = createAgentStreamState("session-a");
  state = reduceAgentStreamEvent(state, streamEvent({ delta: "ephemeral", sequence: 1 }));
  state = reconcileAgentStreamSnapshot(state, snapshot("session-a", [message({
    runId: "run-1",
    externalItemId: "item-1",
    content: "authoritative"
  })]));

  assert.deepEqual(streamingItems(state), []);
  state = reduceAgentStreamEvent(state, streamEvent({ delta: "delayed", sequence: 2 }));
  assert.deepEqual(streamingItems(state), []);
});

test("fresh UI state never restores ephemeral text from a durable snapshot", () => {
  const refreshed = reconcileAgentStreamSnapshot(
    createAgentStreamState("session-a"),
    snapshot("session-a", [message({
      runId: "run-1",
      externalItemId: "item-1",
      content: "durable only"
    })])
  );
  assert.deepEqual(streamingItems(refreshed), []);
});

test("default conversation hides commentary while live streams retain separate commentary and final groups", () => {
  const durable = [
    message({ id: "user", role: "user", content: "Make a part" }),
    message({ id: "commentary", phase: "commentary", content: "I am checking constraints" }),
    message({ id: "final", phase: "final_answer", content: "The part is ready" })
  ];
  assert.deepEqual(buildAgentConversation(durable).map((item) => item.id), ["user", "final"]);

  const state = reduceAgentStreamEvent(
    reduceAgentStreamEvent(
      createAgentStreamState("session-a"),
      streamEvent({ itemId: "commentary-item", phase: "commentary", delta: "checking", sequence: 1 })
    ),
    streamEvent({ itemId: "final-item", phase: "final_answer", delta: "ready", sequence: 2 })
  );
  const groups = splitAgentStreams(streamingItems(state));
  assert.deepEqual(groups.commentary.map((item) => item.text), ["checking"]);
  assert.deepEqual(groups.finalAnswers.map((item) => item.text), ["ready"]);
});

test("run commentary survives stream completion through the durable conversation snapshot", () => {
  const durable = [message({
    id: "durable-commentary",
    runId: "run-1",
    externalItemId: "commentary-item",
    phase: "commentary",
    content: "Durable constraint analysis",
    sequence: 2
  })];
  const whileStreaming = buildRunCommentary(durable, [{
    sessionId: "session-a",
    runId: "run-1",
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: "commentary-item",
    phase: "commentary",
    text: "Ephemeral draft",
    sequence: 2
  }, {
    sessionId: "session-a",
    runId: "run-1",
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: "commentary-live",
    phase: "commentary",
    text: "Still checking geometry",
    sequence: 3
  }], "run-1");

  assert.deepEqual(whileStreaming.map((item) => [item.text, item.streaming]), [
    ["Durable constraint analysis", false],
    ["Still checking geometry", true]
  ]);
  assert.deepEqual(
    buildRunCommentary(durable, [], "run-1").map((item) => item.text),
    ["Durable constraint analysis"]
  );
});

test("durable run commentary fails fast when its persistence identity is incomplete", () => {
  assert.throws(
    () => buildRunCommentary([message({
      id: "broken-commentary",
      runId: "run-1",
      phase: "commentary",
      sequence: 1
    })], [], "run-1"),
    /missing externalItemId/
  );
  assert.throws(
    () => buildRunCommentary([message({
      id: "broken-commentary",
      runId: "run-1",
      externalItemId: "commentary-item",
      phase: "commentary"
    })], [], "run-1"),
    /invalid sequence/
  );
});

test("progress details retain expandable live commentary while workflow summary omits its last tool command", () => {
  const now = "2026-08-12T00:00:00.000Z";
  const html = renderToStaticMarkup(React.createElement(AgentWorkspace, {
    conversation: [message({
      id: "commentary",
      runId: "run-1",
      externalItemId: "commentary-item",
      phase: "commentary",
      content: "I checked the wall thickness.",
      sequence: 2
    })],
    runs: [{
      id: "run-1",
      sessionId: "session-a",
      status: "completed",
      prompt: "Create a part",
      createdAt: now,
      updatedAt: now,
      completedAt: now,
      recoveryStatus: "none"
    }],
    threads: [],
    events: [{
      id: "event-1",
      sessionId: "session-a",
      runId: "run-1",
      type: "agent.tool.completed",
      sequence: 1,
      createdAt: now,
      payload: { command: "cadgen-ax-finalize --session session-a", status: "completed" }
    }],
    workflow: { plans: [], outerIterations: [], pendingVlm: [] },
    validationEvaluations: [],
    validationBatches: [],
    validationChecks: [],
    prompt: "",
    busy: false,
    readOnly: false,
    streams: [],
    onPromptChange: () => undefined,
    onStartRun: () => undefined,
    onStartNewConversation: () => undefined,
    onRetryRun: () => undefined,
    onCancelRun: () => undefined,
    onOpenFullHistory: () => undefined
  }));

  assert.match(html, /data-testid="agent-progress-commentary"/);
  assert.match(html, /<summary><span>Live commentary<\/span>/);
  assert.match(html, /I checked the wall thickness\./);
  assert.match(html, /cadgen-ax-finalize --session session-a/);
  assert.doesNotMatch(workflowSummaryMarkup(html), /cadgen-ax-finalize --session session-a/);
  assert.doesNotMatch(html, /data-testid="validation-report-details"/);
  assert.doesNotMatch(html, /agent\.tool\.completed/);
  assert.doesNotMatch(html, /data-testid="streaming-commentary"/);
  assert.doesNotMatch(html, /agent-debug-menu/);
  assert.doesNotMatch(html, /Full history/);
  assert.doesNotMatch(html, /start-new-agent-conversation/);
  assert.doesNotMatch(html, /New conversation/);
});

test("validation report disclosure exposes the combined report and every batch result", () => {
  const now = "2026-08-12T00:00:00.000Z";
  const kinds = ["structural", "dfm", "vlm"] as const;
  const html = renderToStaticMarkup(React.createElement(AgentWorkspace, {
    conversation: [],
    runs: [{
      id: "run-1",
      sessionId: "session-a",
      status: "completed",
      prompt: "Create a part",
      outputRevisionId: "revision-1",
      createdAt: now,
      updatedAt: now,
      completedAt: now,
      recoveryStatus: "none"
    }],
    threads: [],
    events: [],
    workflow: { plans: [], outerIterations: [], pendingVlm: [] },
    validationEvaluations: [],
    validationBatches: [{
      id: "batch-1",
      sessionId: "session-a",
      runId: "run-1",
      revisionId: "revision-1",
      artifactId: "artifact-1",
      attempt: 1,
      status: "succeeded",
      aggregateReport: {
        contractType: "cadgen-ax.finalization_report.v2",
        passed: false,
        summary: "Combined validation rejected the model."
      },
      createdAt: now,
      settledAt: now
    }],
    validationChecks: kinds.map((kind) => ({
      id: `check-${kind}`,
      batchId: "batch-1",
      sessionId: "session-a",
      kind,
      status: "succeeded" as const,
      inputContract: { contractType: `cadgen-ax.${kind}_input.v1` },
      report: {
        contractType: `cadgen-ax.${kind}_report.v1`,
        passed: kind !== "dfm",
        summary: `${kind} result`,
        ...(kind === "structural" ? {
          checks: [{ name: "topology", passed: true, message: "Mesh is watertight." }]
        } : {}),
        ...(kind === "dfm" ? {
          diagnostics: [{ severity: "error", code: "bridge", message: "Unsupported bridge detected." }],
          failureReport: {
            reason: "dfm_gate_failed",
            summary: "DFM diagnostics rejected the model.",
            nextAction: "refine_source"
          }
        } : {}),
        ...(kind === "vlm" ? {
          scores: { structure: 3, components: 3, proportions: 3 },
          composite: 9,
          score: 1,
          diagnostic: "All requested components are visible."
        } : {})
      },
      passed: kind !== "dfm",
      createdAt: now,
      completedAt: now
    })),
    prompt: "",
    busy: false,
    readOnly: false,
    streams: [],
    onPromptChange: () => undefined,
    onStartRun: () => undefined,
    onStartNewConversation: () => undefined,
    onRetryRun: () => undefined,
    onCancelRun: () => undefined,
    onOpenFullHistory: () => undefined
  }));

  assert.match(html, /data-testid="validation-report-aggregate"/);
  assert.match(html, /<span>Combined report<\/span><small>rejected<\/small>/);
  for (const kind of kinds) {
    assert.match(html, new RegExp(`data-testid="validation-report-${kind}"`));
    assert.match(html, new RegExp(`${kind} result`));
  }
  assert.match(html, /4 reports/);
  assert.match(html, /Decision/);
  assert.match(html, /Unsupported bridge detected\./);
  assert.match(html, /All requested components are visible\./);
  assert.match(html, /Structure/);
  assert.match(html, /Next action: refine source/);
  assert.doesNotMatch(html, /data-testid="validation-batch-summary"/);
  assert.doesNotMatch(html, /Validation batch rejected/);
  assert.doesNotMatch(html, /<pre>/);
  assert.doesNotMatch(html, /cadgen-ax\.dfm_report\.v1/);
  const workflowIndex = html.indexOf('data-testid="workflow-summary"');
  const reportIndex = html.indexOf('data-testid="validation-report-details"');
  const progressIndex = html.indexOf('data-testid="agent-progress-details"');
  assert.ok(workflowIndex >= 0 && workflowIndex < reportIndex);
  assert.ok(reportIndex < progressIndex);
});

test("validation decision view keeps verdict fields and agent diagnostics while dropping raw metadata", () => {
  const decision = validationDecisionView({
    contractType: "cadgen-ax.vlm_judge_report.v1",
    artifactId: "artifact-secret",
    passed: false,
    scores: { structure: 1, components: 3, proportions: 3 },
    composite: 7,
    score: 7 / 9,
    diagnostic: "The support tab is not visible.",
    findings: [{ severity: "error", message: "A requested component is missing." }],
    failureReport: {
      reason: "vlm_score_gate_failed",
      summary: "Every VLM subscore must be at least two.",
      nextAction: "outer_loop_refine_source"
    },
    process: { stdout: "raw process output", stderr: "raw process error" }
  });

  assert.equal(decision.verdict, "Rejected");
  assert.deepEqual(decision.metrics.map((metric) => metric.label), [
    "Composite score",
    "Normalized score",
    "Structure",
    "Components",
    "Proportions"
  ]);
  assert.equal(decision.failureReason, "vlm_score_gate_failed");
  assert.equal(decision.nextAction, "outer_loop_refine_source");
  assert.match(decision.messages.map((item) => item.message).join(" "), /support tab/);
  assert.match(decision.messages.map((item) => item.message).join(" "), /requested component/);
  assert.doesNotMatch(JSON.stringify(decision), /artifact-secret|raw process output|raw process error|contractType/);
});

function workflowSummaryMarkup(html: string): string {
  const start = html.indexOf('data-testid="workflow-summary"');
  const end = html.indexOf('data-testid="agent-progress-details"');
  if (start < 0 || end < 0 || start >= end) {
    throw new Error("Expected workflow summary before progress details.");
  }
  return html.slice(start, end);
}

test("stream identity collision fails fast instead of joining unrelated turns", () => {
  const state = reduceAgentStreamEvent(
    createAgentStreamState("session-a"),
    streamEvent({ delta: "first", sequence: 1 })
  );
  assert.throws(
    () => reduceAgentStreamEvent(state, streamEvent({ turnId: "turn-2", delta: "wrong", sequence: 2 })),
    /Conflicting identity/
  );
});

test("agent diagnostics expose thread, turn, run, status, generation, and recovery identifiers", () => {
  const rows = agentDiagnosticRows(
    [{
      id: "agent-thread-1",
      sessionId: "session-a",
      plane: "modeling",
      ownerId: "session-a",
      externalAgent: "codex",
      externalThreadId: "thread-1",
      status: "ready",
      connectionGeneration: 4,
      createdAt: "2026-08-12T00:00:00.000Z",
      updatedAt: "2026-08-12T00:00:00.000Z"
    }],
    [{
      id: "run-1",
      sessionId: "session-a",
      status: "failed",
      prompt: "fixture",
      createdAt: "2026-08-12T00:00:00.000Z",
      updatedAt: "2026-08-12T00:00:00.000Z",
      externalThreadId: "thread-1",
      externalTurnId: "turn-1",
      recoveryStatus: "unknown_outcome"
    }]
  );

  assert.match(rows[0]?.value ?? "", /mapping agent-thread-1 · external thread-1 · ready · generation 4/);
  assert.match(rows[1]?.value ?? "", /failed · recovery unknown_outcome · thread thread-1 · turn turn-1/);
});

function streamEvent(overrides: Partial<CadAgentStreamEvent> = {}): CadAgentStreamEvent {
  return {
    sessionId: "session-a",
    runId: "run-1",
    threadId: "thread-1",
    turnId: "turn-1",
    itemId: "item-1",
    phase: "final_answer",
    delta: "",
    sequence: 0,
    completed: false,
    ...overrides
  };
}

function snapshot(sessionId: string, conversation: CadConversationMessage[] = []): CadSessionState {
  return { session: { id: sessionId }, conversation } as unknown as CadSessionState;
}

function message(overrides: {
  id?: string;
  role?: CadConversationMessage["role"];
  content?: string;
  runId?: string;
  externalItemId?: string;
  phase?: CadAgentMessagePhase;
  sequence?: number;
} = {}): CadConversationMessage {
  return {
    id: overrides.id ?? "message-1",
    sessionId: "session-a",
    role: overrides.role ?? "assistant",
    content: overrides.content ?? "message",
    createdAt: "2026-08-12T00:00:00.000Z",
    runId: overrides.runId,
    externalItemId: overrides.externalItemId,
    phase: overrides.phase,
    sequence: overrides.sequence
  };
}
