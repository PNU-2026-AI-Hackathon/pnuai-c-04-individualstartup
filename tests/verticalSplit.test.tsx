import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { Window } from "happy-dom";
import { VerticalSplit } from "../ui/src/components/VerticalSplit";

test("vertical split captures the active pointer until the drag ends", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  try {
    await act(async () => {
      root.render(
        <VerticalSplit storageKey="pointer-capture" upperLabel="upper" lowerLabel="lower">
          <div>Upper</div>
          <div>Lower</div>
        </VerticalSplit>
      );
    });

    const split = container.querySelector<HTMLDivElement>(".vertical-split");
    const handle = container.querySelector<HTMLDivElement>(".vertical-split-handle");
    assert.ok(split);
    assert.ok(handle);
    split.getBoundingClientRect = () => new browserWindow.DOMRect(0, 0, 200, 100);

    await act(async () => {
      handle.dispatchEvent(new browserWindow.PointerEvent("pointerdown", {
        bubbles: true,
        button: 0,
        clientY: 40,
        pointerId: 7
      }) as unknown as Event);
    });
    assert.equal(handle.hasPointerCapture(7), true);
    assert.equal(split.classList.contains("is-dragging"), true);

    await act(async () => {
      handle.dispatchEvent(new browserWindow.PointerEvent("pointermove", {
        bubbles: true,
        clientY: 70,
        pointerId: 7
      }) as unknown as Event);
    });
    assert.equal(handle.getAttribute("aria-valuenow"), "70");

    await act(async () => {
      handle.dispatchEvent(new browserWindow.PointerEvent("pointerup", {
        bubbles: true,
        pointerId: 7
      }) as unknown as Event);
    });
    assert.equal(handle.hasPointerCapture(7), false);
    assert.equal(split.classList.contains("is-dragging"), false);
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

test("vertical split restores a finite zero ratio when zero is allowed", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  window.sessionStorage.setItem("zero-ratio", "0");

  try {
    await act(async () => {
      root.render(
        <VerticalSplit
          storageKey="zero-ratio"
          defaultRatio={58}
          minRatio={0}
          maxRatio={100}
          upperLabel="upper"
          lowerLabel="lower"
        >
          <div>Upper</div>
          <div>Lower</div>
        </VerticalSplit>
      );
    });

    const split = container.querySelector<HTMLDivElement>(".vertical-split");
    const handle = container.querySelector<HTMLDivElement>(".vertical-split-handle");
    assert.ok(split);
    assert.ok(handle);
    assert.equal(split.style.getPropertyValue("--split-upper"), "0fr");
    assert.equal(handle.getAttribute("aria-valuenow"), "0");
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

test("vertical split supports keyboard resizing and restores the saved ratio", async () => {
  const browserWindow = installDom();
  const storageKey = "keyboard-persistence";
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  try {
    const split = (
      <VerticalSplit
        storageKey={storageKey}
        defaultRatio={50}
        minRatio={20}
        maxRatio={80}
        upperLabel="Preview"
        lowerLabel="OpenSCAD Source"
      >
        <div>Upper</div>
        <div>Lower</div>
      </VerticalSplit>
    );
    await act(async () => {
      root.render(split);
    });

    const handle = container.querySelector<HTMLDivElement>(".vertical-split-handle");
    assert.ok(handle);

    await pressKey(browserWindow, handle, "ArrowDown");
    assert.equal(handle.getAttribute("aria-valuenow"), "52");

    await pressKey(browserWindow, handle, "ArrowUp", true);
    assert.equal(handle.getAttribute("aria-valuenow"), "42");

    await pressKey(browserWindow, handle, "Home");
    assert.equal(handle.getAttribute("aria-valuenow"), "20");

    await pressKey(browserWindow, handle, "End");
    assert.equal(handle.getAttribute("aria-valuenow"), "80");
    assert.equal(window.sessionStorage.getItem(storageKey), "80");

    await act(async () => {
      root.render(<div>Split unmounted</div>);
    });
    await act(async () => {
      root.render(split);
    });

    const restoredHandle = container.querySelector<HTMLDivElement>(".vertical-split-handle");
    assert.ok(restoredHandle);
    assert.equal(restoredHandle.getAttribute("aria-valuenow"), "80");
  } finally {
    act(() => root.unmount());
    browserWindow.close();
  }
});

async function pressKey(
  browserWindow: Window,
  handle: HTMLElement,
  key: string,
  shiftKey = false
) {
  await act(async () => {
    handle.dispatchEvent(new browserWindow.KeyboardEvent("keydown", {
      bubbles: true,
      key,
      shiftKey
    }) as unknown as Event);
  });
}

function installDom(): Window {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173/" });
  defineGlobal("window", browserWindow);
  defineGlobal("document", browserWindow.document);
  defineGlobal("navigator", browserWindow.navigator);
  defineGlobal("HTMLElement", browserWindow.HTMLElement);
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
