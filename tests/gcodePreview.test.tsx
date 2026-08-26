import assert from "node:assert/strict";
import test from "node:test";
import React, { act, useState } from "react";
import { createRoot } from "react-dom/client";
import * as THREE from "three";
import { Window } from "happy-dom";
import {
  createBedGrid,
  parseRectangularBedShape,
  parseRenderableGCode
} from "../ui/src/MeshPreview";
import {
  PreviewModeSelector,
  type PreviewMode
} from "../ui/src/components/WorkspacePanel";

test("GCodeLoader preview parses travel and extrusion toolpaths", () => {
  const object = parseRenderableGCode([
    "G90",
    "G1 X0 Y0 Z0 F1200",
    "G1 X10 Y0 Z0 E1",
    "G1 X10 Y10 Z0 E2"
  ].join("\n"));

  const lines: THREE.LineSegments[] = [];
  object.traverse((child) => {
    if (child instanceof THREE.LineSegments) lines.push(child);
  });

  assert.equal(object.name, "gcode");
  assert.equal(object.rotation.x, -Math.PI / 2);
  assert.ok(lines.some((line) => line.material instanceof THREE.LineBasicMaterial
    && line.material.name === "extruded"
    && line.geometry.getAttribute("position").count > 0));
  assert.ok(lines.some((line) => line.material instanceof THREE.LineBasicMaterial
    && line.material.name === "path"
    && line.geometry.getAttribute("position").count > 0));
});

test("G-code preview fails fast when no renderable toolpath exists", () => {
  assert.throws(
    () => parseRenderableGCode("M104 S0\nM140 S0"),
    /no renderable G0\/G1 toolpath moves/
  );
  assert.throws(
    () => parseRenderableGCode("G1 Xnot-a-number Y2"),
    /invalid toolpath coordinate/
  );
});

test("Prusa bed_shape metadata creates a 50 by 50 XZ grid", () => {
  const bounds = parseRectangularBedShape([[0, 0], [50, 0], [50, 50], [0, 50]]);
  const grid = createBedGrid(bounds);
  const box = new THREE.Box3().setFromObject(grid);

  assert.deepEqual(bounds, { minX: 0, maxX: 50, minY: 0, maxY: 50 });
  assert.deepEqual(box.min.toArray(), [0, 0, -50]);
  assert.deepEqual(box.max.toArray(), [50, 0, 0]);
  assert.equal(grid.geometry.getAttribute("position").count, 24);
});

test("G-code bed preview rejects malformed or non-rectangular metadata", () => {
  assert.throws(() => parseRectangularBedShape([[0, 0], [50, 0], [25, 50]]), /four corner points/);
  assert.throws(
    () => parseRectangularBedShape([[0, 0], [50, 0], [40, 50], [0, 50]]),
    /axis-aligned rectangular bed/
  );
});

test("preview selector keeps one mode active and disables unavailable G-code", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  function Harness({ gcodeAvailable }: { gcodeAvailable: boolean }) {
    const [mode, setMode] = useState<PreviewMode>("stl");
    return <PreviewModeSelector mode={mode} gcodeAvailable={gcodeAvailable} onChange={setMode} />;
  }

  try {
    await act(async () => root.render(<Harness gcodeAvailable={false} />));
    const buttons = [...container.querySelectorAll<HTMLButtonElement>("button")];
    const stlButton = buttons.find((button) => button.textContent === "STL");
    const gcodeButton = buttons.find((button) => button.textContent === "G-code");
    assert.ok(stlButton);
    assert.ok(gcodeButton);
    assert.equal(stlButton.getAttribute("aria-checked"), "true");
    assert.equal(gcodeButton.getAttribute("aria-checked"), "false");
    assert.equal(gcodeButton.disabled, true);

    await act(async () => root.render(<Harness gcodeAvailable />));
    assert.equal(gcodeButton.disabled, false);
    await act(async () => gcodeButton.click());
    assert.equal(stlButton.getAttribute("aria-checked"), "false");
    assert.equal(gcodeButton.getAttribute("aria-checked"), "true");
    assert.equal(container.querySelectorAll("[aria-checked='true']").length, 1);

    await act(async () => stlButton.click());
    assert.equal(stlButton.getAttribute("aria-checked"), "true");
    assert.equal(gcodeButton.getAttribute("aria-checked"), "false");
    assert.equal(container.querySelectorAll("[aria-checked='true']").length, 1);
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
  defineGlobal("SVGElement", browserWindow.SVGElement);
  defineGlobal("Event", browserWindow.Event);
  defineGlobal("MouseEvent", browserWindow.MouseEvent);
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
