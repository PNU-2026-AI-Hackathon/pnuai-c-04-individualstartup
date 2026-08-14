import assert from "node:assert/strict";
import test from "node:test";
import {
  agentDiagnosticRows,
  buildAgentConversation,
  splitAgentStreams
} from "../ui/src/components/AgentWorkspace";
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
} = {}): CadConversationMessage {
  return {
    id: overrides.id ?? "message-1",
    sessionId: "session-a",
    role: overrides.role ?? "assistant",
    content: overrides.content ?? "message",
    createdAt: "2026-08-12T00:00:00.000Z",
    runId: overrides.runId,
    externalItemId: overrides.externalItemId,
    phase: overrides.phase
  };
}
