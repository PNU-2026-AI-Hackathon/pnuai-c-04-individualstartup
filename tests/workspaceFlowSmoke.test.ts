import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { Window } from "happy-dom";
import { filterSessionsByDeletedIds } from "../ui/src/App";
import { SessionRail } from "../ui/src/components/SessionRail";
import { WorkspacePanel } from "../ui/src/components/WorkspacePanel";
import type { CadRevision, CadSessionListItem, CadSessionState } from "../ui/src/protocol";

test("session rail exposes inline management without top-level Logs navigation", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const calls: string[] = [];
  const sessions = [sessionListItem("active-session"), sessionListItem("other-session")];

  try {
    await act(async () => {
      root.render(React.createElement(SessionRail, {
        sessions,
        activeSessionId: "active-session",
        query: "",
        showArchived: false,
        busy: false,
        open: true,
        view: "workspace",
        onQueryChange: () => undefined,
        onShowArchivedChange: () => undefined,
        onCreateSession: () => { calls.push("create"); },
        onOpenSession: (sessionId: string) => { calls.push(`open:${sessionId}`); },
        onArchiveChange: (sessionId: string, archived: boolean) => { calls.push(`archive:${sessionId}:${archived}`); },
        onRename: (sessionId: string, title: string) => { calls.push(`rename:${sessionId}:${title}`); },
        onDuplicate: (sessionId: string) => { calls.push(`duplicate:${sessionId}`); },
        onDelete: (sessionId: string) => { calls.push(`delete:${sessionId}`); },
        onNavigate: (view: string) => { calls.push(`navigate:${view}`); },
        onClose: () => { calls.push("close"); }
      }));
    });

    assert.doesNotMatch(container.textContent ?? "", /\bLogs\b/);
    assert.doesNotMatch(container.textContent ?? "", /\bManage\b/);

    const menuButton = container.querySelector<HTMLButtonElement>("[aria-label='Session actions for other-session']");
    assert.ok(menuButton);
    await act(async () => menuButton.click());

    for (const label of ["Rename", "Duplicate", "Archive", "Delete"]) {
      assert.match(container.textContent ?? "", new RegExp(label));
    }

    const renameButton = buttonByText(container, "Rename");
    await act(async () => renameButton.click());
    const renameInput = container.querySelector<HTMLInputElement>("[aria-label='Rename session other-session']");
    assert.ok(renameInput);
    const saveButton = container.querySelector<HTMLButtonElement>("[aria-label='Save session title other-session']");
    assert.ok(saveButton);
    await act(async () => saveButton.click());

    assert.deepEqual(calls, ["rename:other-session:other-session"]);
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

test("locally deleted sessions stay hidden when a stale session list arrives", () => {
  const sessions = [
    sessionListItem("active-session"),
    sessionListItem("deleted-session"),
    sessionListItem("other-session")
  ];
  const deletedSessionIds = new Set(["deleted-session"]);

  const visibleSessions = filterSessionsByDeletedIds(sessions, deletedSessionIds);

  assert.deepEqual(
    visibleSessions.map((session) => session.id),
    ["active-session", "other-session"]
  );
});

test("session rail keeps a deleted session out of the DOM after a stale refresh", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const staleSessions = [
    sessionListItem("active-session"),
    sessionListItem("deleted-session"),
    sessionListItem("other-session")
  ];

  function Harness() {
    const [sessions, setSessions] = React.useState(staleSessions);
    const [deletedSessionIds, setDeletedSessionIds] = React.useState<Set<string>>(new Set());
    return React.createElement(SessionRail, {
      sessions: filterSessionsByDeletedIds(sessions, deletedSessionIds),
      activeSessionId: "active-session",
      query: "",
      showArchived: false,
      busy: false,
      open: true,
      view: "workspace",
      onQueryChange: () => undefined,
      onShowArchivedChange: () => undefined,
      onCreateSession: () => undefined,
      onOpenSession: () => undefined,
      onArchiveChange: () => undefined,
      onRename: () => undefined,
      onDuplicate: () => undefined,
      onDelete: (sessionId: string) => {
        setDeletedSessionIds(new Set([sessionId]));
        setSessions(staleSessions);
      },
      onNavigate: () => undefined,
      onClose: () => undefined
    });
  }

  try {
    await act(async () => {
      root.render(React.createElement(Harness));
    });
    assert.match(container.textContent ?? "", /deleted-session/);

    const menuButton = container.querySelector<HTMLButtonElement>("[aria-label='Session actions for deleted-session']");
    assert.ok(menuButton);
    await act(async () => menuButton.click());
    const deleteButton = buttonByText(container, "Delete");
    await act(async () => deleteButton.click());

    assert.doesNotMatch(container.textContent ?? "", /deleted-session/);
    assert.match(container.textContent ?? "", /other-session/);
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

test("workspace preview toolbar export selector does not add result actions under preview", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const state = sampleState();
  const exports: string[] = [];

  try {
    await act(async () => {
      root.render(React.createElement(WorkspacePanel, workspaceProps(state, exports)));
    });

    const exportButton = buttonByText(container, "Export");
    await act(async () => exportButton.click());

    assert.ok(container.textContent?.includes("STL"));
    assert.ok(container.textContent?.includes("STEP"));
    assert.ok(container.textContent?.includes("3MF"));

    const confirmButton = container.querySelector<HTMLButtonElement>(".export-confirm");
    assert.ok(confirmButton);
    await act(async () => confirmButton.click());
    assert.deepEqual(exports, ["stl:revision-1"]);

    assert.doesNotMatch(container.textContent ?? "", /Open file/);
    assert.doesNotMatch(container.textContent ?? "", /Show in Finder/);
    assert.doesNotMatch(container.textContent ?? "", /Copy path/);
    assert.doesNotMatch(container.textContent ?? "", /Export metadata/);
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

function installDom(): Window {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173/sessions/test" });
  defineGlobal("window", browserWindow);
  defineGlobal("document", browserWindow.document);
  defineGlobal("navigator", browserWindow.navigator);
  defineGlobal("HTMLElement", browserWindow.HTMLElement);
  defineGlobal("HTMLButtonElement", browserWindow.HTMLButtonElement);
  defineGlobal("HTMLInputElement", browserWindow.HTMLInputElement);
  defineGlobal("SVGElement", browserWindow.SVGElement);
  defineGlobal("Event", browserWindow.Event);
  defineGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  return browserWindow;
}

function defineGlobal(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value
  });
}

function buttonByText(container: HTMLElement, text: string): HTMLButtonElement {
  const button = [...container.querySelectorAll("button")]
    .find((candidate) => candidate.textContent?.includes(text));
  if (!(button instanceof HTMLButtonElement)) throw new Error(`Button not found: ${text}`);
  return button;
}

function sessionListItem(id: string): CadSessionListItem {
  return {
    id,
    createdAt: "2026-07-30T00:00:00.000Z",
    updatedAt: "2026-07-30T00:00:00.000Z",
    title: id,
    titleSource: "user",
    activeRevisionId: "revision-1",
    selectedRuntime: "openscad-wasm",
    status: "idle",
    archived: false,
    revisionCount: 1,
    artifactCount: 0
  };
}

function workspaceProps(state: CadSessionState, exports: string[]) {
  return {
    state,
    mesh: null,
    runtimeState: "completed" as const,
    busy: false,
    sessionArchived: false,
    source: state.activeRevision?.source ?? "",
    sourceDirty: false,
    showStarterOverlay: false,
    activeRevision: state.activeRevision,
    activeRun: undefined,
    agentPrompt: "",
    onRenderPreview: () => undefined,
    onSaveSource: () => undefined,
    onEditSource: () => undefined,
    onDismissStarterOverlay: () => undefined,
    onPromptChange: () => undefined,
    onStartRun: () => undefined,
    onRetryRun: () => undefined,
    onCancelRun: () => undefined,
    onUpdateParameter: () => undefined,
    onActivateRevision: () => undefined,
    onRestoreRevision: () => undefined,
    onExport: (format: "stl" | "metadata", revisionId?: string) => {
      exports.push(`${format}:${revisionId ?? "none"}`);
    },
    onOpenFullHistory: () => undefined,
    onStartNewConversation: () => undefined
  };
}

function sampleState(): CadSessionState {
  const now = "2026-07-30T00:00:00.000Z";
  const revision: CadRevision = {
    id: "revision-1",
    sessionId: "session-1",
    source: "cube([1, 1, 1]);",
    sourceHash: "source-hash",
    sourceLanguage: "openscad",
    createdAt: now,
    diagnostics: { ok: true, elapsedMs: 8, items: [] },
    parameters: [],
    artifacts: [
      {
        id: "artifact-stl",
        revisionId: "revision-1",
        kind: "stl",
        format: "stl",
        uri: "artifact://stl",
        bytes: 128,
        createdAt: now
      }
    ],
    userEvents: [],
    artifactCount: 1,
    runLinks: []
  };
  return {
    session: {
      id: "session-1",
      createdAt: now,
      updatedAt: now,
      connectedUiClients: 1,
      title: "Session",
      titleSource: "user",
      activeRevisionId: "revision-1",
      selectedRuntime: "openscad-wasm",
      status: "idle",
      revisions: [revision]
    },
    activeRevision: revision,
    messages: [],
    conversation: [],
    agentThreads: [],
    agentRuns: [],
    agentRunEvents: [],
    workflow: { plans: [], outerIterations: [], pendingVlm: [] }
  };
}
