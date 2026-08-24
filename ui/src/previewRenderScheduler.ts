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
 * Coalesces invalidations into one frame and continues only while camera
 * damping changes the view.
 */
export function createDemandRenderScheduler({
  update,
  render,
  requestAnimationFrame,
  cancelAnimationFrame
}: DemandRenderSchedulerOptions): DemandRenderScheduler {
  let pendingFrame: number | null = null;
  let disposed = false;

  const requestRender = () => {
    if (disposed || pendingFrame !== null) return;
    pendingFrame = requestAnimationFrame(() => {
      pendingFrame = null;
      if (disposed) return;
      const dampingChanged = update();
      render();
      if (dampingChanged) requestRender();
    });
  };

  return {
    requestRender,
    dispose() {
      if (disposed) return;
      disposed = true;
      if (pendingFrame !== null) {
        cancelAnimationFrame(pendingFrame);
        pendingFrame = null;
      }
    }
  };
}
