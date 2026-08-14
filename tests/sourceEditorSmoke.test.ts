import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { Window } from "happy-dom";
import { WorkspacePanel } from "../ui/src/components/WorkspacePanel";
import {
  latestValidationEvaluation,
  validationChecksForBatch
} from "../ui/src/components/AgentWorkflow";
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

test("legacy validation evaluations remain compatible when no validation batch exists", async () => {
  const queued = workflowStateWithAcceptedVlm();
  queued.validationEvaluations = [validationEvaluation(queued, "queued")];
  const harness = mountWorkspace({
    sessionId: queued.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: queued
  });

  assert.ok(workflowStep(harness.container, "VLM").classList.contains("workflow-step-active"));
  assert.ok(workflowStep(harness.container, "Complete").classList.contains("workflow-step-pending"));
  assert.match(harness.container.textContent ?? "", /VLM evaluation queued/);

  const running = workflowStateWithAcceptedVlm();
  running.validationEvaluations = [validationEvaluation(running, "running")];
  harness.rerender({
    sessionId: running.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: running
  });
  assert.ok(workflowStep(harness.container, "VLM").classList.contains("workflow-step-active"));
  assert.match(harness.container.textContent ?? "", /VLM evaluation running/);

  const passed = workflowStateWithPendingVlm();
  passed.validationEvaluations = [validationEvaluation(passed, "succeeded", true)];
  harness.rerender({
    sessionId: passed.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: passed
  });
  assert.ok(workflowStep(harness.container, "VLM").classList.contains("workflow-step-pass"));
  assert.ok(workflowStep(harness.container, "Complete").classList.contains("workflow-step-pass"));
  assert.match(harness.container.textContent ?? "", /VLM evaluation passed/);
  assert.doesNotMatch(harness.container.textContent ?? "", /Pending VLM/);

  const rejected = workflowStateWithAcceptedVlm();
  rejected.validationEvaluations = [validationEvaluation(rejected, "succeeded", false)];
  harness.rerender({
    sessionId: rejected.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: rejected
  });
  assert.ok(workflowStep(harness.container, "VLM").classList.contains("workflow-step-fail"));
  assert.ok(workflowStep(harness.container, "Complete").classList.contains("workflow-step-fail"));
  assert.match(harness.container.textContent ?? "", /score 0\.4/);

  const failed = workflowStateWithAcceptedVlm();
  failed.validationEvaluations = [validationEvaluation(failed, "failed")];
  harness.rerender({
    sessionId: failed.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: failed
  });
  assert.ok(workflowStep(harness.container, "VLM").classList.contains("workflow-step-fail"));
  assert.ok(workflowStep(harness.container, "Complete").classList.contains("workflow-step-fail"));
  assert.match(harness.container.textContent ?? "", /transport failed/);
  harness.cleanup();
});

test("latest validation batch drives parallel checks, pass, rejection, and operational failure", async () => {
  const running = workflowStateWithValidationBatch("running", {
    structural: "running",
    dfm: "queued",
    vlm: "running"
  });
  const harness = mountWorkspace({
    sessionId: running.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: running
  });
  for (const label of ["Structural", "DFM", "VLM"]) {
    assert.ok(workflowStep(harness.container, label).classList.contains("workflow-step-active"));
  }
  assert.ok(workflowStep(harness.container, "Complete").classList.contains("workflow-step-pending"));
  assert.match(harness.container.textContent ?? "", /Validation batch running/);
  assert.match(harness.container.textContent ?? "", /Structural: running/);
  assert.match(harness.container.textContent ?? "", /DFM: queued/);

  const passed = workflowStateWithValidationBatch("succeeded", {
    structural: "succeeded",
    dfm: "succeeded",
    vlm: "succeeded"
  }, true);
  harness.rerender({
    sessionId: passed.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: passed
  });
  for (const label of ["Structural", "DFM", "VLM", "Complete"]) {
    assert.ok(workflowStep(harness.container, label).classList.contains("workflow-step-pass"));
  }
  assert.match(harness.container.textContent ?? "", /Validation batch passed/);

  const rejected = workflowStateWithValidationBatch("succeeded", {
    structural: "succeeded",
    dfm: "succeeded",
    vlm: "succeeded"
  }, false, "structural");
  harness.rerender({
    sessionId: rejected.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: rejected
  });
  assert.ok(workflowStep(harness.container, "Structural").classList.contains("workflow-step-fail"));
  assert.ok(workflowStep(harness.container, "DFM").classList.contains("workflow-step-pass"));
  assert.ok(workflowStep(harness.container, "Complete").classList.contains("workflow-step-fail"));
  assert.match(harness.container.textContent ?? "", /Validation batch rejected/);
  assert.match(harness.container.textContent ?? "", /wall too thin/);

  const failed = workflowStateWithValidationBatch("failed", {
    structural: "succeeded",
    dfm: "failed",
    vlm: "succeeded"
  });
  harness.rerender({
    sessionId: failed.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state: failed
  });
  assert.ok(workflowStep(harness.container, "Structural").classList.contains("workflow-step-pass"));
  assert.ok(workflowStep(harness.container, "DFM").classList.contains("workflow-step-fail"));
  assert.ok(workflowStep(harness.container, "Complete").classList.contains("workflow-step-fail"));
  assert.match(harness.container.textContent ?? "", /Validation batch operational failure/);
  assert.match(harness.container.textContent ?? "", /slicer crashed/);
  harness.cleanup();
});

test("validation batch selection ignores stale revisions and older attempts", async () => {
  const state = workflowStateWithValidationBatch("queued", {
    structural: "queued",
    dfm: "queued",
    vlm: "queued"
  });
  const current = state.validationBatches[0];
  if (!current) throw new Error("Batch fixture missing current batch.");
  const older = { ...current, id: "batch-older", attempt: current.attempt - 1, status: "succeeded" as const,
    aggregateReport: { passed: true } };
  const stale = { ...current, id: "batch-stale", revisionId: "revision-stale", attempt: 99,
    status: "failed" as const, aggregateReport: undefined };
  state.validationBatches = [older, stale, current];
  state.validationChecks.push(
    ...validationChecksForFixture(state, older, {
      structural: "succeeded", dfm: "succeeded", vlm: "succeeded"
    }, true),
    ...validationChecksForFixture(state, stale, {
      structural: "succeeded", dfm: "failed", vlm: "succeeded"
    }, true)
  );
  const harness = mountWorkspace({
    sessionId: state.session.id,
    source: "cube([1, 1, 1]);",
    runtimeState: "completed",
    state
  });
  assert.match(harness.container.textContent ?? "", /Validation batch queued/);
  assert.doesNotMatch(harness.container.textContent ?? "", /operational failure/);
  assert.ok(workflowStep(harness.container, "Structural").classList.contains("workflow-step-active"));
  assert.ok(workflowStep(harness.container, "Complete").classList.contains("workflow-step-pending"));
  harness.cleanup();
});

test("validation batch check graph fails fast instead of inventing missing state", () => {
  const state = workflowStateWithValidationBatch("queued", {
    structural: "queued", dfm: "queued", vlm: "queued"
  });
  assert.throws(
    () => validationChecksForBatch(state.validationChecks.slice(0, 2), state.validationBatches[0]!.id),
    /exactly one structural, DFM, and VLM check/
  );
});

test("latest validation evaluation is scoped to the run output revision before ordering attempts", () => {
  const state = workflowStateWithAcceptedVlm();
  const run = state.agentRuns[0];
  if (!run) throw new Error("Validation ordering fixture requires a run.");
  run.outputRevisionId = "revision-current";
  const previousRevision = {
    ...validationEvaluation(state, "succeeded", true),
    id: "evaluation-previous-attempt-2",
    revisionId: "revision-previous",
    attempt: 2,
    createdAt: "2026-07-30T00:00:02.000Z"
  };
  const currentRevision = {
    ...validationEvaluation(state, "queued"),
    id: "evaluation-current-attempt-1",
    revisionId: "revision-current",
    attempt: 1,
    createdAt: "2026-07-30T00:00:01.000Z"
  };

  assert.equal(
    latestValidationEvaluation(
      [previousRevision, currentRevision],
      run.id,
      run.outputRevisionId
    )?.id,
    currentRevision.id
  );
  assert.equal(
    latestValidationEvaluation([previousRevision], run.id, run.outputRevisionId),
    undefined
  );
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
    validationEvaluations: [],
    validationBatches: [],
    validationChecks: [],
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

function validationEvaluation(
  state: CadSessionState,
  status: "queued" | "running" | "succeeded" | "failed",
  passed?: boolean
): CadSessionState["validationEvaluations"][number] {
  const now = "2026-07-30T00:00:00.000Z";
  const run = state.agentRuns[0];
  if (!run || !state.activeRevision) throw new Error("Validation fixture requires run and revision.");
  return {
    id: `evaluation-${status}`,
    sessionId: state.session.id,
    runId: run.id,
    revisionId: state.activeRevision.id,
    artifactId: "artifact-1",
    kind: "vlm",
    attempt: 2,
    status,
    inputContract: { contractType: "cadastrophe.vlm_evaluation_input.v1" },
    report: status === "succeeded" ? { contractType: "cadastrophe.vlm_judge_report.v1" } : undefined,
    passed: status === "succeeded" ? passed : undefined,
    score: status === "succeeded" ? (passed ? 0.92 : 0.4) : undefined,
    passThreshold: 0.8,
    error: status === "failed" ? "transport failed" : undefined,
    createdAt: now,
    startedAt: status === "queued" ? undefined : now,
    completedAt: status === "succeeded" || status === "failed" ? now : undefined
  };
}

type FixtureCheckStatuses = Record<"structural" | "dfm" | "vlm", "queued" | "running" | "succeeded" | "failed">;

function workflowStateWithValidationBatch(
  status: "queued" | "running" | "succeeded" | "failed",
  checkStatuses: FixtureCheckStatuses,
  passed?: boolean,
  rejectedKind?: "structural" | "dfm" | "vlm"
): CadSessionState {
  const state = workflowStateWithAcceptedVlm();
  const revisionId = state.activeRevision?.id;
  const run = state.agentRuns[0];
  if (!revisionId || !run) throw new Error("Validation batch fixture requires run and revision.");
  run.outputRevisionId = revisionId;
  run.status = status === "succeeded" && passed === true ? "completed" : "running";
  const batch: CadSessionState["validationBatches"][number] = {
    id: `batch-${status}-${passed ?? "none"}`,
    sessionId: state.session.id,
    runId: run.id,
    revisionId,
    artifactId: "artifact-1",
    attempt: 2,
    status,
    aggregateReport: status === "succeeded"
      ? {
          contractType: "cadastrophe.finalization_report.v2",
          passed,
          failureReport: passed === false
            ? {
                contractType: "cadastrophe.failure_report.v1",
                reason: "validation_batch_rejected",
                summary: "wall too thin",
                nextAction: "outer_loop_refine_source"
              }
            : null
        }
      : undefined,
    createdAt: "2026-07-30T00:00:01.000Z",
    startedAt: status === "queued" ? undefined : "2026-07-30T00:00:02.000Z",
    settledAt: status === "succeeded" || status === "failed"
      ? "2026-07-30T00:00:03.000Z"
      : undefined
  };
  state.validationBatches = [batch];
  state.validationChecks = validationChecksForFixture(state, batch, checkStatuses, true, rejectedKind);
  return state;
}

function validationChecksForFixture(
  state: CadSessionState,
  batch: CadSessionState["validationBatches"][number],
  statuses: FixtureCheckStatuses,
  passed = true,
  rejectedKind?: "structural" | "dfm" | "vlm"
): CadSessionState["validationChecks"] {
  return (["structural", "dfm", "vlm"] as const).map((kind) => {
    const status = statuses[kind];
    const checkPassed = status === "succeeded" ? passed && kind !== rejectedKind : undefined;
    return {
      id: `${batch.id}-${kind}`,
      batchId: batch.id,
      sessionId: state.session.id,
      kind,
      status,
      inputContract: { contractType: `cadastrophe.${kind}_input.v1` },
      report: status === "succeeded"
        ? { contractType: `cadastrophe.${kind}_report.v1`, passed: checkPassed }
        : undefined,
      passed: checkPassed,
      error: status === "failed" ? "slicer crashed" : undefined,
      createdAt: batch.createdAt,
      startedAt: status === "queued" ? undefined : batch.startedAt,
      completedAt: status === "succeeded" || status === "failed" ? batch.settledAt : undefined
    };
  });
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
    validationEvaluations: [],
    validationBatches: [],
    validationChecks: [],
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
