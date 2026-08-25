export interface DemandRenderScheduler {
  requestRender(): void;
  dispose(): void;
}

export interface DemandRenderSchedulerOptions {
  update(): boolean;
  render(): void;
  requestAnimationFrame(callback: FrameRequestCallback): number;
  cancelAnimationFrame(handle: number): void;
}

/**
 * Coalesces invalidations into one RAF callback. While a callback is running,
 * requests raised by OrbitControls are folded into that callback; another
 * frame is scheduled only when damping reports that the view changed.
 *
 * A thrown update/render error is allowed to propagate, but the pending handle
 * is always cleared so the scheduler never remains stuck after the failure.
 */
export function createDemandRenderScheduler({
  update,
  render,
  requestAnimationFrame,
  cancelAnimationFrame
}: DemandRenderSchedulerOptions): DemandRenderScheduler {
  let pendingRAFHandle: number | null = null;
  let disposed = false;

  const requestRender = () => {
    if (disposed || pendingRAFHandle !== null) return;
    pendingRAFHandle = requestAnimationFrame(() => {
      if (disposed) return;
      let dampingChanged = false;
      try {
        dampingChanged = update();
        render();
      } finally {
        pendingRAFHandle = null;
      }
      if (dampingChanged) requestRender();
    });
  };

  return {
    requestRender,
    dispose() {
      if (disposed) return;
      disposed = true;
      if (pendingRAFHandle !== null) {
        cancelAnimationFrame(pendingRAFHandle);
        pendingRAFHandle = null;
      }
    }
  };
}
