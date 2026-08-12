import type {
  CadAgentMessagePhase,
  CadAgentStreamEvent,
  CadSessionState
} from "../protocol";

export interface CadAgentStreamingItem {
  sessionId: string;
  runId: string;
  threadId: string;
  turnId: string;
  itemId: string;
  phase: CadAgentMessagePhase;
  text: string;
  sequence: number;
}

export interface CadAgentStreamState {
  sessionId: string;
  items: ReadonlyMap<string, CadAgentStreamingItem>;
  completedItemKeys: ReadonlySet<string>;
}

export function createAgentStreamState(sessionId: string): CadAgentStreamState {
  return {
    sessionId,
    items: new Map(),
    completedItemKeys: new Set()
  };
}

export function reduceAgentStreamEvent(
  state: CadAgentStreamState,
  event: CadAgentStreamEvent
): CadAgentStreamState {
  if (event.sessionId !== state.sessionId) return state;
  assertStreamEvent(event);
  const key = streamItemKey(event.runId, event.itemId);

  if (event.completed) {
    if (!state.items.has(key) && state.completedItemKeys.has(key)) return state;
    const items = new Map(state.items);
    const completedItemKeys = new Set(state.completedItemKeys);
    items.delete(key);
    completedItemKeys.add(key);
    return { ...state, items, completedItemKeys };
  }

  if (state.completedItemKeys.has(key)) return state;
  const current = state.items.get(key);
  if (current) {
    assertSameStreamIdentity(current, event);
    if (event.sequence <= current.sequence) return state;
  }

  const items = new Map(state.items);
  items.set(key, {
    sessionId: event.sessionId,
    runId: event.runId,
    threadId: event.threadId,
    turnId: event.turnId,
    itemId: event.itemId,
    phase: event.phase,
    text: `${current?.text ?? ""}${event.delta}`,
    sequence: event.sequence
  });
  return { ...state, items };
}

/**
 * Removes items represented by an authoritative durable snapshot. Recording
 * tombstones prevents a delayed delta from recreating an already committed
 * message after the snapshot wins the race with the completed stream event.
 */
export function reconcileAgentStreamSnapshot(
  state: CadAgentStreamState,
  snapshot: CadSessionState
): CadAgentStreamState {
  if (snapshot.session.id !== state.sessionId) {
    return createAgentStreamState(snapshot.session.id);
  }
  const durableKeys = snapshot.conversation.flatMap((message) =>
    message.runId && message.externalItemId
      ? [streamItemKey(message.runId, message.externalItemId)]
      : []
  );
  if (!durableKeys.length) return state;

  const items = new Map(state.items);
  const completedItemKeys = new Set(state.completedItemKeys);
  let changed = false;
  for (const key of durableKeys) {
    if (items.delete(key)) changed = true;
    if (!completedItemKeys.has(key)) {
      completedItemKeys.add(key);
      changed = true;
    }
  }
  return changed ? { ...state, items, completedItemKeys } : state;
}

export function streamingItems(state: CadAgentStreamState): CadAgentStreamingItem[] {
  return [...state.items.values()].sort((left, right) => left.sequence - right.sequence);
}

function streamItemKey(runId: string, itemId: string): string {
  return `${runId}\u0000${itemId}`;
}

function assertStreamEvent(event: CadAgentStreamEvent): void {
  for (const [name, value] of Object.entries({
    sessionId: event.sessionId,
    runId: event.runId,
    threadId: event.threadId,
    turnId: event.turnId,
    itemId: event.itemId
  })) {
    if (!value) throw new Error(`agent_stream_event.${name} must be non-empty.`);
  }
  if (event.phase !== "commentary" && event.phase !== "final_answer") {
    throw new Error(`Unsupported agent_stream_event phase: ${String(event.phase)}`);
  }
  if (!Number.isSafeInteger(event.sequence) || event.sequence < 0) {
    throw new Error("agent_stream_event.sequence must be a non-negative safe integer.");
  }
  if (typeof event.delta !== "string" || typeof event.completed !== "boolean") {
    throw new Error("agent_stream_event delta/completed fields have invalid types.");
  }
}

function assertSameStreamIdentity(
  current: CadAgentStreamingItem,
  event: CadAgentStreamEvent
): void {
  if (
    current.threadId !== event.threadId ||
    current.turnId !== event.turnId ||
    current.phase !== event.phase
  ) {
    throw new Error(`Conflicting identity for streamed item ${event.runId}/${event.itemId}.`);
  }
}
