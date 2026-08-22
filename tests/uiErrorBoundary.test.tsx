import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { Window } from "happy-dom";
import { UiErrorBoundary } from "../ui/src/UiErrorBoundary";

test("UI rendering failures remain visible instead of unmounting the root", () => {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173/" });
  installDom(browserWindow);
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const originalConsoleError = console.error;
  console.error = () => undefined;

  function BrokenPreview(): React.JSX.Element {
    throw new Error("WebGL preview initialization failed");
  }

  try {
    act(() => {
      root.render(
        <UiErrorBoundary scope="Preview">
          <BrokenPreview />
        </UiErrorBoundary>
      );
    });
    const boundary = container.querySelector("[data-testid='preview-error-boundary']");
    assert.ok(boundary);
    assert.match(boundary.textContent ?? "", /Preview render failed/);
    assert.match(boundary.textContent ?? "", /WebGL preview initialization failed/);
  } finally {
    console.error = originalConsoleError;
    act(() => root.unmount());
    browserWindow.close();
  }
});

function installDom(browserWindow: Window) {
  defineGlobal("window", browserWindow);
  defineGlobal("document", browserWindow.document);
  defineGlobal("HTMLElement", browserWindow.HTMLElement);
  defineGlobal("SVGElement", browserWindow.SVGElement);
  defineGlobal("Event", browserWindow.Event);
  defineGlobal("IS_REACT_ACT_ENVIRONMENT", true);
}

function defineGlobal(name: string, value: unknown) {
  Object.defineProperty(globalThis, name, {
    configurable: true,
    writable: true,
    value
  });
}
