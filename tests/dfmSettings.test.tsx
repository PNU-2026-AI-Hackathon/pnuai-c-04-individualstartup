import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { Window } from "happy-dom";
import type {
  DfmProfileDocument,
  DfmProfileValidation,
  DfmSettings as DfmSettingsState,
  DfmSettingsBackendClient,
  PrusaSlicerValidation
} from "../ui/src/backendClient";
import { DfmSettings, type DfmSettingsDialog } from "../ui/src/components/DfmSettings";
import { WorkflowRunSummary, type WorkflowRunView } from "../ui/src/components/AgentWorkflow";
import { macAppExecutablePath, parseDfmProfile, updateDfmProfileValue } from "../ui/src/dfmProfile";
import type { CadAgentRun } from "../ui/src/protocol";

const VALID_PROFILE = `# DFM profile
printer_technology = FFF
nozzle_diameter = 0.4
filament_diameter = 1.75
layer_height = 0.2
gcode_flavor = reprap
support_material = 1
fill_density = 20%
machine_max_acceleration_x = 9000,1000
perimeter_speed = 60
fill_pattern = grid
`;

test("profile parser classifies typed values and refuses missing required settings", () => {
  const parsed = parseDfmProfile(VALID_PROFILE);
  assert.deepEqual(parsed.errors, []);
  assert.equal(parsed.entries.find((entry) => entry.key === "support_material")?.valueType, "boolean");
  assert.equal(parsed.entries.find((entry) => entry.key === "fill_density")?.valueType, "percent");
  assert.equal(parsed.entries.find((entry) => entry.key === "machine_max_acceleration_x")?.valueType, "multi");
  assert.equal(parsed.entries.find((entry) => entry.key === "fill_pattern")?.valueType, "enum");
  assert.equal(parsed.entries.find((entry) => entry.key === "perimeter_speed")?.category, "Speed");

  const missing = parseDfmProfile("printer_technology = FFF\n");
  assert.ok(missing.errors.some((error) => error.includes("nozzle_diameter")));
  assert.throws(() => updateDfmProfileValue(VALID_PROFILE, "missing_key", "1"), /exactly one/);
  assert.equal(
    macAppExecutablePath("/Applications/PrusaSlicer.app"),
    "/Applications/PrusaSlicer.app/Contents/MacOS/PrusaSlicer"
  );
});

test("workflow summary exposes the DFM profile hash, key settings, and G-code artifact", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const run: CadAgentRun = {
    id: "run-1",
    sessionId: "session-1",
    status: "completed",
    prompt: "printable bracket",
    createdAt: "2026-08-12T00:00:00.000Z",
    updatedAt: "2026-08-12T00:00:00.000Z",
    recoveryStatus: "none"
  };
  const view: WorkflowRunView = {
    stage: "VLM pending",
    finalizationStatus: "waiting for VLM",
    iterations: [],
    latestDfmReport: {
      contractType: "cadastrophe.dfm_report.v1",
      passed: true,
      checks: [{ id: "overhang" }],
      diagnostics: [],
      profileHash: "profile-hash-abcdef",
      gcodeArtifactId: "gcode-artifact-12345",
      keySettings: { layer_height: "0.2", fill_density: "20%" }
    }
  };

  try {
    await act(async () => root.render(<WorkflowRunSummary run={run} view={view} />));
    assert.match(container.textContent ?? "", /DFM passed/);
    assert.match(container.textContent ?? "", /profile profile-hash-abcdef/);
    assert.match(container.textContent ?? "", /G-code gcode-ar/);
    assert.match(container.textContent ?? "", /layer height0\.2/);
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

test("DFM settings persist executable and profile changes and recover them after remount", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const backend = new MemoryDfmBackend();
  const dialog = new MemoryDialog();
  let root = createRoot(container);

  try {
    await act(async () => root.render(<DfmSettings backend={backend} dialog={dialog} />));
    await flush();

    assert.match(container.textContent ?? "", /Version2\.9\.6/);
    assert.match(container.textContent ?? "", /profile-hash-1/);
    assert.ok(container.querySelector<HTMLSelectElement>("select[aria-label='support_material']"));
    assert.ok(container.querySelector<HTMLInputElement>("input[aria-label='fill_density']"));
    assert.ok(container.querySelector<HTMLSelectElement>("select[aria-label='fill_pattern']"));

    const executableInput = requiredElement<HTMLInputElement>(container, "input[aria-label='PrusaSlicer executable path']");
    await changeInput(executableInput, "/opt/PrusaSlicer/bin/prusa-slicer");
    assert.match(container.textContent ?? "", /Needs validation/);
    await clickButton(container, "Validate");
    assert.equal(backend.validatedExecutablePath, "/opt/PrusaSlicer/bin/prusa-slicer");
    assert.match(container.textContent ?? "", /Version2\.9\.6/);
    await clickButton(container, "Save executable");
    assert.equal(backend.savedExecutablePath, "/opt/PrusaSlicer/bin/prusa-slicer");
    assert.match(container.textContent ?? "", /PrusaSlicer executable saved/);

    const layerHeight = requiredElement<HTMLInputElement>(container, "input[aria-label='layer_height']");
    await changeInput(layerHeight, "0.28");
    assert.match(container.textContent ?? "", /Unsaved/);
    await clickButton(container, "Save profile");
    assert.match(backend.savedProfile, /layer_height = 0\.28/);

    await clickButton(container, "Import INI");
    assert.equal(backend.importedPath, "/profiles/imported.ini");
    await clickButton(container, "Export INI");
    assert.equal(backend.exportedPath, "/profiles/exported.ini");

    await clickButton(container, "Advanced key/value");
    const advancedEditor = requiredElement<HTMLTextAreaElement>(container, "textarea[aria-label='Complete profile.ini']");
    await changeField(advancedEditor, "printer_technology = FFF\n");
    const saveProfileButton = buttonByText(container, "Save profile");
    assert.equal(saveProfileButton.disabled, true);
    assert.match(container.textContent ?? "", /Required setting “nozzle_diameter” is missing/);

    await changeField(advancedEditor, backend.savedProfile);

    await act(async () => root.unmount());
    container.replaceChildren();
    root = createRoot(container);
    await act(async () => root.render(<DfmSettings backend={backend} dialog={dialog} />));
    await flush();

    assert.equal(
      requiredElement<HTMLInputElement>(container, "input[aria-label='PrusaSlicer executable path']").value,
      "/opt/PrusaSlicer/bin/prusa-slicer"
    );
    assert.equal(requiredElement<HTMLInputElement>(container, "input[aria-label='layer_height']").value, "0.28");
    assert.match(container.textContent ?? "", /profile-hash-2/);
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

class MemoryDfmBackend implements DfmSettingsBackendClient {
  savedExecutablePath = "/Applications/PrusaSlicer.app/Contents/MacOS/PrusaSlicer";
  savedProfile = VALID_PROFILE;
  private profileRevision = 1;
  validatedExecutablePath: string | null = null;
  importedPath: string | null = null;
  exportedPath: string | null = null;

  async getDfmSettings(): Promise<DfmSettingsState> {
    return {
      prusaslicerExecutable: this.savedExecutablePath,
      executableValidation: { path: this.savedExecutablePath, version: "2.9.6" },
      profile: this.profileDocument()
    };
  }

  async validatePrusaSlicerExecutable(input: { path: string }): Promise<PrusaSlicerValidation> {
    this.validatedExecutablePath = input.path;
    return { path: input.path, version: "2.9.6" };
  }

  async savePrusaSlicerExecutable(input: { path: string }): Promise<PrusaSlicerValidation> {
    this.savedExecutablePath = input.path;
    return this.validatePrusaSlicerExecutable(input);
  }

  async validateDfmProfile(): Promise<DfmProfileValidation> {
    return { hash: `profile-hash-${this.profileRevision}`, keySettings: keySettings() };
  }

  async saveDfmProfile(input: { contents: string }): Promise<DfmProfileDocument> {
    this.savedProfile = input.contents;
    this.profileRevision += 1;
    return this.profileDocument();
  }

  async importDfmProfile(input: { path: string }) {
    this.importedPath = input.path;
    return { contents: this.savedProfile, sourcePath: input.path };
  }

  async exportDfmProfile(input: { path: string; contents: string }) {
    assert.ok(input.contents);
    this.exportedPath = input.path;
    return { path: input.path };
  }

  async restoreDefaultDfmProfile(): Promise<DfmProfileDocument> {
    this.savedProfile = VALID_PROFILE;
    this.profileRevision += 1;
    return this.profileDocument();
  }

  private profileDocument(): DfmProfileDocument {
    return {
      contents: this.savedProfile,
      hash: `profile-hash-${this.profileRevision}`,
      keySettings: keySettings()
    };
  }
}

class MemoryDialog implements DfmSettingsDialog {
  async chooseExecutable() { return "/Applications/PrusaSlicer.app"; }
  async chooseProfileToImport() { return "/profiles/imported.ini"; }
  async chooseProfileExportPath() { return "/profiles/exported.ini"; }
}

function keySettings(): Record<string, string> {
  return { layer_height: "0.2 mm", nozzle_diameter: "0.4 mm", fill_density: "20%" };
}

async function changeInput(input: HTMLInputElement, value: string) {
  await changeField(input, value);
}

async function changeField(input: HTMLInputElement | HTMLTextAreaElement, value: string) {
  await act(async () => {
    const propsKey = Object.keys(input).find((key) => key.startsWith("__reactProps$"));
    const onChange = propsKey
      ? (input as unknown as Record<string, { onChange?: (event: { target: typeof input }) => void }>)[propsKey]?.onChange
      : undefined;
    if (!onChange) throw new Error("Rendered field does not expose a React onChange handler.");
    onChange({ target: Object.assign(input, { value }) });
  });
}

async function clickButton(container: HTMLElement, text: string) {
  const button = [...container.querySelectorAll("button")].find((candidate) => candidate.textContent?.includes(text));
  if (!(button instanceof HTMLButtonElement)) throw new Error(`Button not found: ${text}`);
  await act(async () => button.click());
  await flush();
}

function requiredElement<T extends Element>(container: HTMLElement, selector: string): T {
  const element = container.querySelector<T>(selector);
  if (!element) throw new Error(`Element not found: ${selector}`);
  return element;
}

function buttonByText(container: HTMLElement, text: string): HTMLButtonElement {
  const button = [...container.querySelectorAll("button")].find((candidate) => candidate.textContent?.includes(text));
  if (!(button instanceof HTMLButtonElement)) throw new Error(`Button not found: ${text}`);
  return button;
}

async function flush() {
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
}

function installDom(): Window {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173/sessions/test?view=settings" });
  defineGlobal("window", browserWindow);
  defineGlobal("document", browserWindow.document);
  defineGlobal("navigator", browserWindow.navigator);
  defineGlobal("HTMLElement", browserWindow.HTMLElement);
  defineGlobal("HTMLButtonElement", browserWindow.HTMLButtonElement);
  defineGlobal("HTMLInputElement", browserWindow.HTMLInputElement);
  defineGlobal("HTMLTextAreaElement", browserWindow.HTMLTextAreaElement);
  defineGlobal("SVGElement", browserWindow.SVGElement);
  defineGlobal("Event", browserWindow.Event);
  defineGlobal("InputEvent", browserWindow.InputEvent);
  defineGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  return browserWindow;
}

function defineGlobal(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
}
