import assert from "node:assert/strict";
import test from "node:test";
import * as THREE from "three";
import { Window } from "happy-dom";
import {
  PREVIEW_PIXEL_RATIO_LIMIT,
  mountPreview,
  parseRenderableGCode,
  type PreviewRuntime
} from "../ui/src/MeshPreview";
import type { CadMesh } from "../ui/src/protocol";

const triangleMesh: CadMesh = {
  vertices: [0, 0, 0, 10, 0, 0, 0, 10, 0],
  normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
  indices: [0, 1, 2]
};

test("preview renders initial STL, controls, and resize only on demand", () => {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173" });
  const domContainer = browserWindow.document.createElement("div");
  browserWindow.document.body.appendChild(domContainer);
  const container = domContainer as unknown as HTMLDivElement;
  setElementSize(container, 320, 240);
  const harness = createPreviewRuntimeHarness(browserWindow, 2);

  const dispose = mountPreview({
    activeMesh: triangleMesh,
    bedBounds: null,
    container,
    gcodeObject: null,
    matcap: new THREE.Texture(),
    mode: "stl"
  }, harness.runtime);

  assert.equal(harness.pixelRatios.at(-1), PREVIEW_PIXEL_RATIO_LIMIT);
  assert.equal(1 - PREVIEW_PIXEL_RATIO_LIMIT ** 2 / 2 ** 2, 0.4375);
  assert.deepEqual(harness.sizes.at(-1), [320, 240]);
  assert.equal(harness.animationFrames.pendingCount(), 1);
  harness.animationFrames.flushNext();
  assert.equal(harness.rendered.length, 1);
  assert.equal(harness.animationFrames.pendingCount(), 0);
  const stlResources = trackSceneResources(harness.rendered[0].scene);
  assert.ok(Number(container.dataset.cameraDistance) > 0);
  assert.match(container.dataset.cameraPosition ?? "", /^-?[\d.]+,-?[\d.]+,-?[\d.]+$/);

  harness.controls.dampingUpdates.push(true, true, false);
  harness.controls.dispatchEvent({ type: "change" });
  harness.controls.dispatchEvent({ type: "change" });
  assert.equal(harness.animationFrames.pendingCount(), 1);
  harness.animationFrames.flushNext();
  harness.animationFrames.flushNext();
  harness.animationFrames.flushNext();
  assert.equal(harness.rendered.length, 4);
  assert.equal(harness.animationFrames.pendingCount(), 0);

  const sizeCountBeforeUnchangedResize = harness.sizes.length;
  harness.resize();
  assert.equal(harness.sizes.length, sizeCountBeforeUnchangedResize);
  assert.equal(harness.animationFrames.pendingCount(), 0);

  setElementSize(container, 200, 120);
  harness.resize();
  assert.deepEqual(harness.sizes.at(-1), [200, 120]);
  assert.equal(harness.animationFrames.pendingCount(), 1);
  harness.animationFrames.flushNext();
  assert.equal(harness.rendered.at(-1)?.camera.aspect, 200 / 120);

  harness.controls.dispatchEvent({ type: "change" });
  const sizeCountBeforeDispose = harness.sizes.length;
  dispose();
  dispose();
  assert.equal(harness.animationFrames.pendingCount(), 0);
  assert.equal(harness.observerDisconnected, true);
  assert.equal(harness.controls.listenerCount, 0);
  assert.equal(harness.controls.disposed, true);
  assert.equal(harness.rendererDisposed, true);
  assert.equal(container.querySelector("canvas"), null);
  stlResources.assertDisposed();
  harness.resize();
  assert.equal(harness.sizes.length, sizeCountBeforeDispose);
  browserWindow.close();
});

test("new G-code content receives an immediate frame with bed-aware framing", () => {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173" });
  const domContainer = browserWindow.document.createElement("div");
  browserWindow.document.body.appendChild(domContainer);
  const container = domContainer as unknown as HTMLDivElement;
  setElementSize(container, 300, 180);
  // DPR 1 covers the uncapped path; the STL lifecycle test above covers capping.
  const harness = createPreviewRuntimeHarness(browserWindow, 1);
  const gcodeObject = parseRenderableGCode("G90\nG1 X0 Y0 Z0\nG1 X10 Y10 Z1 E1");

  const dispose = mountPreview({
    activeMesh: null,
    bedBounds: { minX: 0, maxX: 20, minY: 0, maxY: 20 },
    container,
    gcodeObject,
    matcap: null,
    mode: "gcode"
  }, harness.runtime);

  assert.equal(harness.pixelRatios.at(-1), 1);
  assert.equal(harness.animationFrames.pendingCount(), 1);
  harness.animationFrames.flushNext();
  assert.equal(harness.rendered.length, 1);
  const gcodeResources = trackSceneResources(harness.rendered[0].scene);
  assert.ok(harness.rendered[0].scene.getObjectByName("gcode"));
  assert.equal(harness.rendered[0].scene.children.some((child) => child instanceof THREE.Mesh), false);
  assert.ok(harness.controls.target.distanceTo(new THREE.Vector3(10, 0.5, -10)) < 1e-6);
  assert.equal(harness.animationFrames.pendingCount(), 0);

  dispose();
  assert.equal(harness.controls.listenerCount, 0);
  assert.equal(harness.observerDisconnected, true);
  gcodeResources.assertDisposed();
  browserWindow.close();
});

test("returning to STL creates a mesh, reframes it, and disposes its Matcap", () => {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173" });
  const domContainer = browserWindow.document.createElement("div");
  browserWindow.document.body.appendChild(domContainer);
  const container = domContainer as unknown as HTMLDivElement;
  setElementSize(container, 300, 180);
  const restoredHarness = createPreviewRuntimeHarness(browserWindow, 1.5);
  const restoredTexture = new THREE.Texture();
  let restoredTextureDisposed = false;
  restoredTexture.addEventListener("dispose", () => { restoredTextureDisposed = true; });
  const translatedMesh: CadMesh = {
    ...triangleMesh,
    vertices: triangleMesh.vertices.map((coordinate, index) => (
      coordinate + (index % 3 === 0 ? 100 : 0)
    ))
  };
  const disposeRestored = mountPreview({
    activeMesh: translatedMesh,
    bedBounds: null,
    container,
    gcodeObject: null,
    matcap: restoredTexture,
    mode: "stl"
  }, restoredHarness.runtime);
  restoredHarness.animationFrames.flushNext();
  assert.ok(restoredHarness.rendered[0].scene.children.some((child) => child instanceof THREE.Mesh));
  assert.ok(restoredHarness.controls.target.distanceTo(new THREE.Vector3(105, 5, 0)) < 1e-6);
  assert.equal(restoredHarness.animationFrames.pendingCount(), 0);
  disposeRestored();
  assert.equal(restoredTextureDisposed, true);
  assert.equal(restoredHarness.controls.listenerCount, 0);
  assert.equal(restoredHarness.observerDisconnected, true);
  browserWindow.close();
});

test("initialization failure releases renderer, controls, observer, DOM, and model resources", () => {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173" });
  const domContainer = browserWindow.document.createElement("div");
  browserWindow.document.body.appendChild(domContainer);
  const container = domContainer as unknown as HTMLDivElement;
  setElementSize(container, 320, 240);
  const observerFailure = new Error("Resize observer failed during initialization.");
  const harness = createPreviewRuntimeHarness(browserWindow, 1, observerFailure);
  const texture = new THREE.Texture();
  let textureDisposed = false;
  texture.addEventListener("dispose", () => { textureDisposed = true; });

  assert.throws(() => mountPreview({
    activeMesh: triangleMesh,
    bedBounds: null,
    container,
    gcodeObject: null,
    matcap: texture,
    mode: "stl"
  }, harness.runtime), observerFailure);

  assert.equal(harness.animationFrames.pendingCount(), 0);
  assert.equal(harness.observerDisconnected, true);
  assert.equal(harness.controls.listenerCount, 0);
  assert.equal(harness.controls.disposed, true);
  assert.equal(harness.rendererDisposed, true);
  assert.equal(textureDisposed, true);
  assert.equal(container.querySelector("canvas"), null);
  browserWindow.close();
});

function createPreviewRuntimeHarness(
  browserWindow: Window,
  devicePixelRatio: number,
  observeFailure?: Error
) {
  const animationFrames = createAnimationFrameHarness();
  const canvas = browserWindow.document.createElement("canvas") as unknown as HTMLCanvasElement;
  const pixelRatios: number[] = [];
  const sizes: Array<[number, number]> = [];
  const rendered: Array<{ scene: THREE.Scene; camera: THREE.PerspectiveCamera }> = [];
  let rendererDisposed = false;
  let observerDisconnected = false;
  let resizeCallback: ResizeObserverCallback | null = null;
  let camera: THREE.PerspectiveCamera | null = null;

  class FakeControls extends THREE.EventDispatcher<{ change: object }> {
    target = new THREE.Vector3();
    minDistance = 0;
    maxDistance = Infinity;
    enableDamping = false;
    enableRotate = false;
    enableZoom = false;
    enablePan = false;
    dampingFactor = 0;
    zoomSpeed = 0;
    rotateSpeed = 0;
    dampingUpdates: boolean[] = [];
    disposed = false;

    update() {
      const changed = this.dampingUpdates.shift() ?? false;
      if (changed) this.dispatchEvent({ type: "change" });
      return changed;
    }

    getDistance() {
      if (!camera) throw new Error("Preview camera was not created.");
      return camera.position.distanceTo(this.target);
    }

    dispose() {
      this.disposed = true;
    }

    get listenerCount() {
      const listeners = (this as unknown as { _listeners?: Record<string, unknown[]> })._listeners;
      return listeners?.change?.length ?? 0;
    }
  }

  const controls = new FakeControls();
  const runtime: PreviewRuntime = {
    devicePixelRatio,
    createRenderer: () => ({
      domElement: canvas,
      setPixelRatio: (ratio: number) => { pixelRatios.push(ratio); },
      setSize: (width: number, height: number) => { sizes.push([width, height]); },
      render: (scene: THREE.Scene, renderCamera: THREE.Camera) => {
        if (!(renderCamera instanceof THREE.PerspectiveCamera)) {
          throw new Error("Preview did not use a perspective camera.");
        }
        rendered.push({ scene, camera: renderCamera });
      },
      dispose: () => { rendererDisposed = true; }
    } as unknown as THREE.WebGLRenderer),
    createControls: (createdCamera) => {
      camera = createdCamera;
      return controls as unknown as import("three/addons/controls/OrbitControls.js").OrbitControls;
    },
    createResizeObserver: (callback) => {
      resizeCallback = callback;
      return {
        observe() {
          if (observeFailure) throw observeFailure;
        },
        disconnect() { observerDisconnected = true; }
      };
    },
    requestAnimationFrame: animationFrames.request,
    cancelAnimationFrame: animationFrames.cancel
  };

  return {
    animationFrames,
    controls,
    pixelRatios,
    sizes,
    rendered,
    runtime,
    resize() {
      if (!resizeCallback) throw new Error("Resize observer was not created.");
      resizeCallback([], {} as ResizeObserver);
    },
    get observerDisconnected() { return observerDisconnected; },
    get rendererDisposed() { return rendererDisposed; }
  };
}

function createAnimationFrameHarness() {
  let nextHandle = 1;
  const callbacks = new Map<number, FrameRequestCallback>();
  return {
    request(callback: FrameRequestCallback) {
      const handle = nextHandle;
      nextHandle += 1;
      callbacks.set(handle, callback);
      return handle;
    },
    cancel(handle: number) {
      callbacks.delete(handle);
    },
    pendingCount() {
      return callbacks.size;
    },
    flushNext() {
      const entry = callbacks.entries().next().value as [number, FrameRequestCallback] | undefined;
      if (!entry) throw new Error("No animation frame is pending.");
      callbacks.delete(entry[0]);
      entry[1](0);
    }
  };
}

function setElementSize(element: HTMLDivElement, width: number, height: number) {
  Object.defineProperty(element, "clientWidth", { configurable: true, value: width });
  Object.defineProperty(element, "clientHeight", { configurable: true, value: height });
}

function trackSceneResources(scene: THREE.Scene) {
  const geometries = new Set<THREE.BufferGeometry>();
  const materials = new Set<THREE.Material>();
  const disposedGeometries = new Set<THREE.BufferGeometry>();
  const disposedMaterials = new Set<THREE.Material>();
  scene.traverse((child) => {
    if ("geometry" in child && child.geometry instanceof THREE.BufferGeometry) {
      geometries.add(child.geometry);
    }
    if (!("material" in child)) return;
    const childMaterials = Array.isArray(child.material) ? child.material : [child.material];
    for (const material of childMaterials) {
      if (material instanceof THREE.Material) materials.add(material);
    }
  });
  for (const geometry of geometries) {
    geometry.addEventListener("dispose", () => { disposedGeometries.add(geometry); });
  }
  for (const material of materials) {
    material.addEventListener("dispose", () => { disposedMaterials.add(material); });
  }
  return {
    assertDisposed() {
      assert.equal(disposedGeometries.size, geometries.size);
      assert.equal(disposedMaterials.size, materials.size);
    }
  };
}
