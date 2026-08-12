import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { Window } from "happy-dom";
import { WorkspacePanel } from "../ui/src/components/WorkspacePanel";
import type { CadRevision, CadSessionState } from "../ui/src/protocol";
import type { OpenscadRuntimeState } from "../ui/src/runtime/openscadRuntime";

test("source editor click and focus keep the workspace mounted", async () => {
  const harness = mountWorkspace({
    sessionId: "fresh-sample",
    source: "cube([1, 1, 1]);",
    showStarterOverlay: true
  });
  let dismissCount = 0;
  harness.updateHandlers({
    onDismissStarterOverlay: () => {
      dismissCount += 1;
    }
  });

  await clickAndFocus(harness.sourceEditor());

  assert.ok(dismissCount >= 1);
  assert.ok(harness.container.querySelector(".workspace"));
  assert.ok(harness.sourceEditor());
  harness.cleanup();
});

test("starter source is displayed as overlay while the editable source stays empty", async () => {
  const starterSource = "cube([1, 1, 1]);";
  const harness = mountWorkspace({
    sessionId: "empty-starter",
    source: "",
    starterSource,
    showStarterOverlay: true
  });

  assert.equal(harness.sourceEditor().value, "");
  assert.equal(
    harness.container.querySelector(".source-starter-overlay")?.textContent,
    starterSource
  );

  await clickAndFocus(harness.sourceEditor());

  assert.equal(harness.sourceEditor().value, "");
  harness.cleanup();
});

test("starter preview does not advance workflow progress for an empty session", async () => {
  const harness = mountWorkspace({
    sessionId: "empty-preview",
    source: "",
    emptySession: true,
    runtimeState: "completed",
    starterSource: "cube([1, 1, 1]);",
    showStarterOverlay: true
  });

  const steps = [...harness.container.querySelectorAll<HTMLElement>(".workflow-step")];
  const previewStep = steps.find((step) => step.textContent?.includes("Preview"));
  const structuralStep = steps.find((step) => step.textContent?.includes("Structural"));

  assert.ok(previewStep?.classList.contains("workflow-step-pending"));
  assert.ok(structuralStep?.classList.contains("workflow-step-pending"));
  harness.cleanup();
});

test("finalize command completion does not mark workflow complete before VLM acceptance", async () => {
  const harness = mountWorkspace({
    sessionId: "pending-vlm",
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: workflowStateWithPendingVlm()
  });

  const vlmStep = workflowStep(harness.container, "VLM");
  const completeStep = workflowStep(harness.container, "Complete");

  assert.ok(vlmStep.classList.contains("workflow-step-active"));
  assert.ok(completeStep.classList.contains("workflow-step-pending"));
  assert.doesNotMatch(harness.container.textContent ?? "", /\bFinalized\b/);

  harness.rerender({
    sessionId: "accepted-vlm",
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: workflowStateWithAcceptedVlm()
  });

  assert.ok(workflowStep(harness.container, "VLM").classList.contains("workflow-step-pass"));
  assert.ok(workflowStep(harness.container, "Complete").classList.contains("workflow-step-pass"));
  harness.cleanup();
});

test("source editor click after session switch uses the new editor instance", async () => {
  const dismissedSessions: string[] = [];
  const harness = mountWorkspace({
    sessionId: "existing-a",
    source: "cube([1, 1, 1]);",
    showStarterOverlay: true,
    onDismissStarterOverlay: () => dismissedSessions.push("existing-a")
  });

  await clickAndFocus(harness.sourceEditor());
  harness.rerender({
    sessionId: "existing-b",
    source: "sphere(r = 4);",
    showStarterOverlay: true,
    onDismissStarterOverlay: () => dismissedSessions.push("existing-b")
  });
  await clickAndFocus(harness.sourceEditor());

  assert.equal(dismissedSessions[0], "existing-a");
  assert.equal(dismissedSessions.at(-1), "existing-b");
  assert.equal(harness.sourceEditor().value, "sphere(r = 4);");
  assert.ok(harness.container.querySelector(".workspace-inspector"));
  harness.cleanup();
});

test("source editor click during render keeps render UI and editor mounted", async () => {
  const harness = mountWorkspace({
    sessionId: "rendering-session",
    source: "cylinder(h = 8, r = 2);",
    runtimeState: "rendering"
  });

  await clickAndFocus(harness.sourceEditor());

  assert.match(harness.container.textContent ?? "", /Rendering/);
  assert.ok(harness.container.querySelector(".preview-pane"));
  assert.ok(harness.sourceEditor());
  harness.cleanup();
});

test("source editor focus handler failures stay local to the editor pane", async () => {
  const originalConsoleError = console.error;
  console.error = () => undefined;
  const harness = mountWorkspace({
    sessionId: "focus-error",
    source: "cube([2, 2, 2]);",
    showStarterOverlay: true,
    onDismissStarterOverlay: () => {
      throw new Error("starter overlay state failed");
    }
  });

  try {
    await clickAndFocus(harness.sourceEditor());

    const fallback = harness.container.querySelector(".source-editor-fallback");
    assert.ok(fallback);
    assert.match(fallback.textContent ?? "", /starter overlay state failed/);
    assert.ok(harness.container.querySelector(".preview-pane"));
    assert.ok(harness.container.querySelector(".workspace-inspector"));
  } finally {
    console.error = originalConsoleError;
    harness.cleanup();
  }
});

type WorkspaceHarnessOptions = {
  sessionId: string;
  source: string;
  state?: CadSessionState;
  emptySession?: boolean;
  starterSource?: string;
  runtimeState?: OpenscadRuntimeState;
  showStarterOverlay?: boolean;
  onDismissStarterOverlay?: () => void;
};

type WorkspaceHarness = {
  container: HTMLElement;
  rerender: (options: WorkspaceHarnessOptions) => void;
  sourceEditor: () => HTMLTextAreaElement;
  updateHandlers: (handlers: Partial<WorkspaceHandlers>) => void;
  cleanup: () => void;
};

type WorkspaceHandlers = {
  onDismissStarterOverlay: () => void;
};

function mountWorkspace(options: WorkspaceHarnessOptions): WorkspaceHarness {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173/sessions/test" });
  installDom(browserWindow);
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  let handlers: WorkspaceHandlers = {
    onDismissStarterOverlay: options.onDismissStarterOverlay ?? (() => undefined)
  };

  const render = (nextOptions: WorkspaceHarnessOptions) => {
    act(() => {
      root.render(React.createElement(WorkspacePanel, workspaceProps(nextOptions, handlers)));
    });
  };

  render(options);

  return {
    container,
    rerender: render,
    sourceEditor: () => {
      const editor = container.querySelector<HTMLTextAreaElement>("[data-testid='source-editor']");
      if (!editor) throw new Error("Source editor was not mounted.");
      return editor;
    },
    updateHandlers: (nextHandlers) => {
      handlers = { ...handlers, ...nextHandlers };
      render(options);
    },
    cleanup: () => {
      act(() => root.unmount());
      browserWindow.close();
    }
  };
}

function installDom(browserWindow: Window) {
  defineGlobal("window", browserWindow);
  defineGlobal("document", browserWindow.document);
  defineGlobal("navigator", browserWindow.navigator);
  defineGlobal("HTMLElement", browserWindow.HTMLElement);
  defineGlobal("HTMLTextAreaElement", browserWindow.HTMLTextAreaElement);
  defineGlobal("SVGElement", browserWindow.SVGElement);
  defineGlobal("Event", browserWindow.Event);
  defineGlobal("FocusEvent", browserWindow.FocusEvent);
  defineGlobal("MouseEvent", browserWindow.MouseEvent);
  defineGlobal("IS_REACT_ACT_ENVIRONMENT", true);
}

function defineGlobal(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value
  });
}

async function clickAndFocus(editor: HTMLTextAreaElement) {
  await act(async () => {
    editor.click();
    editor.focus();
    editor.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
  });
}

function workspaceProps(options: WorkspaceHarnessOptions, handlers: WorkspaceHandlers) {
  const state = options.state
    ?? (options.emptySession ? emptyState(options.sessionId) : sampleState(options.sessionId, options.source));
  return {
    state,
    mesh: null,
    runtimeState: options.runtimeState ?? "idle",
    busy: false,
    sessionArchived: false,
    source: options.source,
    starterSource: options.starterSource,
    sourceDirty: false,
    showStarterOverlay: Boolean(options.showStarterOverlay),
    activeRevision: state.activeRevision,
    activeRun: undefined,
    agentPrompt: "",
    onRenderPreview: () => undefined,
    onSaveSource: () => undefined,
    onEditSource: () => undefined,
    onDismissStarterOverlay: options.onDismissStarterOverlay ?? handlers.onDismissStarterOverlay,
    onPromptChange: () => undefined,
    onStartRun: () => undefined,
    onRetryRun: () => undefined,
    onCancelRun: () => undefined,
    onUpdateParameter: () => undefined,
    onActivateRevision: () => undefined,
    onRestoreRevision: () => undefined,
    onExport: () => undefined,
    onOpenFullHistory: () => undefined,
    onStartNewConversation: () => undefined
  };
}

function workflowStep(container: HTMLElement, label: string): HTMLElement {
  const step = [...container.querySelectorAll<HTMLElement>(".workflow-step")]
    .find((candidate) => candidate.textContent?.includes(label));
  if (!step) throw new Error(`Workflow step not found: ${label}`);
  return step;
}

function emptyState(sessionId: string): CadSessionState {
  const now = "2026-07-30T00:00:00.000Z";
  return {
    session: {
      id: sessionId,
      createdAt: now,
      updatedAt: now,
      connectedUiClients: 1,
      title: `Session ${sessionId}`,
      titleSource: "user",
      selectedRuntime: "openscad-wasm",
      status: "idle",
      revisions: []
    },
    activeRevision: undefined,
    messages: [],
    conversation: [],
    agentThreads: [],
    agentRuns: [],
    agentRunEvents: [],
    workflow: {
      plans: [],
      outerIterations: [],
      pendingVlm: []
    }
  };
}

function workflowStateWithPendingVlm(): CadSessionState {
  const state = sampleState("pending-vlm", "cube([1, 1, 1]);");
  const now = "2026-07-30T00:00:00.000Z";
  state.agentRuns = [{
    id: "run-1",
    sessionId: state.session.id,
    inputRevisionId: state.activeRevision?.id,
    status: "running",
    prompt: "Create a cube.",
    createdAt: now,
    updatedAt: now,
    recoveryStatus: "none"
  }];
  state.agentRunEvents = [{
    id: "event-1",
    sessionId: state.session.id,
    runId: "run-1",
    revisionId: state.activeRevision?.id,
    type: "agent.tool.completed",
    sequence: 1,
    createdAt: now,
    payload: { command: "cadastrophe-finalize", status: "completed" }
  }];
  state.workflow = {
    plans: [workflowPlan(state, "run-1", now)],
    outerIterations: [],
    pendingVlm: [{
      runId: "run-1",
      artifactId: "artifact-1",
      contract: {
        contractType: "cadastrophe.vlm_judge.v1",
        runId: "run-1",
        artifactId: "artifact-1"
      },
      passThreshold: 0.8,
      createdAt: now
    }]
  };
  return state;
}

function workflowStateWithAcceptedVlm(): CadSessionState {
  const state = workflowStateWithPendingVlm();
  const now = "2026-07-30T00:00:00.000Z";
  state.agentRuns = state.agentRuns.map((run) => ({
    ...run,
    status: "completed",
    completedAt: now,
    updatedAt: now
  }));
  state.workflow.pendingVlm = [];
  state.workflow.outerIterations = [{
    id: "outer-1",
    runId: "run-1",
    iteration: 1,
    revisionId: state.activeRevision?.id,
    structuralReport: {
      contractType: "cadastrophe.structural_report.v1",
      passed: true
    },
    vlmReport: {
      contractType: "cadastrophe.vlm_report.v1",
      passed: true,
      score: 0.92
    },
    passed: true,
    createdAt: now
  }];
  return state;
}

function workflowPlan(state: CadSessionState, runId: string, now: string): CadSessionState["workflow"]["plans"][number] {
  return {
    runId,
    revisionId: state.activeRevision?.id,
    sourceLanguage: "openscad",
    createdAt: now,
    plan: {
      schemaVersion: "cad_model_plan.v1",
      summary: "Cube fixture.",
      mainComponent: {
        name: "cube_fixture",
        purpose: "Exercise workflow progress display."
      },
      supportingComponents: [],
      expectedAspectRatio: { x: 1, y: 1, z: 1, tolerance: 0.25 },
      sourceLanguage: "openscad",
      runtimeConstraints: {
        runtime: "openscad-wasm",
        mainComponentAnnotation: "cube_fixture"
      }
    }
  };
}

function sampleState(sessionId: string, source: string): CadSessionState {
  const now = "2026-07-30T00:00:00.000Z";
  const revision = sampleRevision(sessionId, source, now);
  return {
    session: {
      id: sessionId,
      createdAt: now,
      updatedAt: now,
      connectedUiClients: 1,
      title: `Session ${sessionId}`,
      titleSource: "user",
      activeRevisionId: revision.id,
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
    workflow: {
      plans: [],
      outerIterations: [],
      pendingVlm: []
    }
  };
}

function sampleRevision(sessionId: string, source: string, now: string): CadRevision {
  return {
    id: `${sessionId}-revision`,
    sessionId,
    sourceHash: `${sessionId}-source-hash`,
    sourceLanguage: "openscad",
    source,
    parameters: [],
    createdAt: now,
    diagnostics: { ok: true, elapsedMs: 0, items: [] },
    artifactCount: 0,
    artifacts: [],
    userEvents: [],
    runLinks: []
  };
}
