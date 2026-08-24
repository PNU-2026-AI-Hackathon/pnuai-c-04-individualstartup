import assert from "node:assert/strict";
import test from "node:test";
import { createDemandRenderScheduler } from "../ui/src/previewRenderScheduler";

test("demand scheduler coalesces requests and stops after damping", () => {
  const animationFrames = createAnimationFrameHarness();
  const dampingUpdates = [false, true, true, false];
  let renderCount = 0;
  const scheduler = createDemandRenderScheduler({
    update: () => dampingUpdates.shift() ?? false,
    render: () => { renderCount += 1; },
    requestAnimationFrame: animationFrames.request,
    cancelAnimationFrame: animationFrames.cancel
  });

  scheduler.requestRender();
  scheduler.requestRender();
  assert.equal(animationFrames.pendingCount(), 1);
  animationFrames.flushNext();
  assert.equal(renderCount, 1);
  assert.equal(animationFrames.pendingCount(), 0);

  scheduler.requestRender();
  animationFrames.flushNext();
  assert.equal(animationFrames.pendingCount(), 1);
  animationFrames.flushNext();
  assert.equal(animationFrames.pendingCount(), 1);
  animationFrames.flushNext();
  assert.equal(renderCount, 4);
  assert.equal(animationFrames.pendingCount(), 0);
});

test("demand scheduler cancels pending work and rejects post-disposal requests", () => {
  const animationFrames = createAnimationFrameHarness();
  let renderCount = 0;
  const scheduler = createDemandRenderScheduler({
    update: () => false,
    render: () => { renderCount += 1; },
    requestAnimationFrame: animationFrames.request,
    cancelAnimationFrame: animationFrames.cancel
  });

  scheduler.requestRender();
  const callback = animationFrames.peekNext();
  scheduler.dispose();
  assert.equal(animationFrames.pendingCount(), 0);
  assert.deepEqual(animationFrames.cancelledHandles, [1]);

  callback(0);
  scheduler.requestRender();
  assert.equal(renderCount, 0);
  assert.equal(animationFrames.pendingCount(), 0);
});

function createAnimationFrameHarness() {
  let nextHandle = 1;
  const callbacks = new Map<number, FrameRequestCallback>();
  const cancelledHandles: number[] = [];
  return {
    cancelledHandles,
    request(callback: FrameRequestCallback) {
      const handle = nextHandle;
      nextHandle += 1;
      callbacks.set(handle, callback);
      return handle;
    },
    cancel(handle: number) {
      cancelledHandles.push(handle);
      callbacks.delete(handle);
    },
    pendingCount() {
      return callbacks.size;
    },
    peekNext() {
      const callback = callbacks.values().next().value as FrameRequestCallback | undefined;
      if (!callback) throw new Error("No animation frame is pending.");
      return callback;
    },
    flushNext() {
      const entry = callbacks.entries().next().value as [number, FrameRequestCallback] | undefined;
      if (!entry) throw new Error("No animation frame is pending.");
      callbacks.delete(entry[0]);
      entry[1](0);
    }
  };
}
