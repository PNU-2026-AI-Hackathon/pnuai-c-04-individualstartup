import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { Window } from "happy-dom";
import { filterSessionsByDeletedIds, latestRenderableGcodeArtifact } from "../ui/src/App";
import { Timeline } from "../ui/src/components/RevisionPanels";
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

test("session rail combines revisions with accessible icon-only navigation and keyboard resizing", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const state = sampleState();

  try {
    await act(async () => {
      root.render(React.createElement(SessionRail, {
        sessions: [sessionListItem("session-1")],
        activeSessionId: "session-1",
        query: "",
        showArchived: false,
        busy: false,
        open: true,
        view: "workspace",
        revisionState: state,
        revisionsReadOnly: false,
        sourceDirty: false,
        onQueryChange: () => undefined,
        onShowArchivedChange: () => undefined,
        onCreateSession: () => undefined,
        onOpenSession: () => undefined,
        onArchiveChange: () => undefined,
        onRename: () => undefined,
        onDuplicate: () => undefined,
        onDelete: () => undefined,
        onNavigate: () => undefined,
        onActivateRevision: () => undefined,
        onRestoreRevision: () => undefined,
        onClose: () => undefined
      }));
    });

    assert.ok(container.querySelector(".session-rail-sessions"));
    assert.ok(container.querySelector(".session-rail-revisions .timeline"));
    for (const label of ["Model", "Files", "Settings"]) {
      const button = container.querySelector<HTMLButtonElement>(`.session-rail-nav [aria-label='${label}']`);
      assert.ok(button);
      assert.equal(button.textContent, "");
      assert.equal(button.title, label);
    }

    const splitter = container.querySelector<HTMLElement>("[aria-label='Resize Sessions and Revisions']");
    assert.ok(splitter);
    const before = Number(splitter.getAttribute("aria-valuenow"));
    await act(async () => {
      splitter.dispatchEvent(
        new browserWindow.KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }) as unknown as Event
      );
    });
    assert.equal(Number(splitter.getAttribute("aria-valuenow")), before + 2);

    const split = container.querySelector<HTMLElement>(".session-rail-split");
    assert.ok(split);
    split.getBoundingClientRect = () => ({
      x: 0,
      y: 0,
      top: 0,
      right: 272,
      bottom: 400,
      left: 0,
      width: 272,
      height: 400,
      toJSON: () => ({})
    });
    await act(async () => {
      splitter.dispatchEvent(new browserWindow.PointerEvent(
        "pointerdown", { button: 0, clientY: 120, pointerId: 7, bubbles: true }
      ) as unknown as Event);
    });
    await act(async () => {
      splitter.dispatchEvent(new browserWindow.PointerEvent(
        "pointermove", { clientY: 280, pointerId: 7, bubbles: true }
      ) as unknown as Event);
    });
    assert.equal(Number(splitter.getAttribute("aria-valuenow")), 70);
    await act(async () => {
      splitter.dispatchEvent(new browserWindow.PointerEvent(
        "pointerup", { pointerId: 7, bubbles: true }
      ) as unknown as Event);
    });
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

test("timeline renders an empty state when revision state is unavailable", () => {
  for (const state of [undefined, {} as CadSessionState]) {
    const html = renderToStaticMarkup(React.createElement(Timeline, {
      state,
      busy: false,
      readOnly: false,
      sourceDirty: false,
      onActivate: () => undefined,
      onRestore: () => undefined
    }));

    assert.match(html, /No revisions yet\./);
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

test("G-code preview uses the latest available artifact from the active revision", () => {
  const revision = sampleState().activeRevision;
  assert.ok(revision);
  const artifact = (id: string, createdAt: string, unavailable?: "deleted" | "missing") => ({
    id,
    revisionId: revision.id,
    revisionHash: "b".repeat(64),
    kind: "gcode" as const,
    format: "gcode",
    uri: `artifact://${id}`,
    createdAt,
    deletedAt: unavailable === "deleted" ? createdAt : undefined,
    missingAt: unavailable === "missing" ? createdAt : undefined
  });
  const oldArtifact = artifact("gcode-old", "2026-07-30T00:00:01.000Z");
  revision.artifacts.push(
    oldArtifact,
    artifact("gcode-new-missing", "2026-07-30T00:00:03.000Z", "missing"),
    artifact("gcode-new", "2026-07-30T00:00:02.000Z")
  );

  assert.equal(latestRenderableGcodeArtifact(revision)?.id, "gcode-new");
  revision.artifacts = revision.artifacts.filter((candidate) => candidate.id !== "gcode-new");
  oldArtifact.deletedAt = "2026-07-30T00:00:04.000Z";
  assert.equal(latestRenderableGcodeArtifact(revision), undefined);
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

    assert.doesNotMatch(container.textContent ?? "", /\bRender\b/);
    const previewModes = container.querySelector("[aria-label='Preview type']");
    assert.ok(previewModes);
    const gcodeButton = buttonByText(previewModes as HTMLElement, "G-code");
    assert.equal(gcodeButton.disabled, true);
    assert.equal(previewModes.querySelectorAll("[aria-checked='true']").length, 1);

    const exportButton = container.querySelector<HTMLButtonElement>("[aria-label='Export current model']");
    assert.ok(exportButton);
    assert.equal(exportButton.textContent, "");
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

test("workspace keeps agent and empty parameters visible at responsive height", async () => {
  const browserWindow = installDom(1000, 800);
  const style = document.createElement("style");
  style.textContent = [
    readFileSync(new URL("../ui/src/styles/workspace.css", import.meta.url), "utf8"),
    readFileSync(new URL("../ui/src/styles/responsive.css", import.meta.url), "utf8")
  ].join("\n");
  document.head.appendChild(style);
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const state = sampleState();

  try {
    await act(async () => {
      root.render(React.createElement(WorkspacePanel, workspaceProps(state, [])));
    });

    const workspace = container.querySelector<HTMLElement>(".workspace");
    const agent = container.querySelector<HTMLElement>(".inspector-agent .agent-workspace");
    const parameters = container.querySelector<HTMLElement>(".inspector-parameters .parameters-panel");
    const inspector = container.querySelector<HTMLElement>(".workspace-inspector");
    assert.ok(workspace);
    assert.ok(agent);
    assert.ok(parameters);
    assert.ok(inspector);
    assert.match(parameters.textContent ?? "", /No parameters are available for this revision\./);
    assert.equal(window.getComputedStyle(workspace).overflowY, "auto");
    assert.equal(window.getComputedStyle(inspector).height, "620px");
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

function installDom(width = 1024, height = 768): Window {
  const browserWindow = new Window({
    url: "http://127.0.0.1:5173/sessions/test",
    width,
    height
  });
  defineGlobal("window", browserWindow);
  defineGlobal("document", browserWindow.document);
  defineGlobal("navigator", browserWindow.navigator);
  defineGlobal("HTMLElement", browserWindow.HTMLElement);
  defineGlobal("HTMLButtonElement", browserWindow.HTMLButtonElement);
  defineGlobal("HTMLInputElement", browserWindow.HTMLInputElement);
  defineGlobal("SVGElement", browserWindow.SVGElement);
  defineGlobal("Event", browserWindow.Event);
  defineGlobal("PointerEvent", browserWindow.PointerEvent);
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
    gcode: null,
    gcodeArtifactId: undefined,
    gcodeBedShape: undefined,
    runtimeState: "completed" as const,
    busy: false,
    sessionArchived: false,
    source: state.activeRevision?.source ?? "",
    sourceDirty: false,
    showStarterOverlay: false,
    activeRevision: state.activeRevision,
    activeRun: undefined,
    agentPrompt: "",
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
        revisionHash: "a".repeat(64),
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
    validationEvaluations: [],
    validationBatches: [],
    validationChecks: [],
    workflow: { plans: [], outerIterations: [], pendingVlm: [] }
  };
}
