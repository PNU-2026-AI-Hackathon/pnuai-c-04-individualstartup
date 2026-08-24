import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import * as THREE from "three";
import { Window } from "happy-dom";
import {
  COLD_METAL_MATCAP_URL,
  MeshPreview,
  createPreviewScene,
  createStlPreviewObject,
  disposeObject,
  loadColdMetalMatcap
} from "../ui/src/MeshPreview";
import { UiErrorBoundary } from "../ui/src/UiErrorBoundary";
import type { CadMesh } from "../ui/src/protocol";

const triangleMesh: CadMesh = {
  vertices: [0, 0, 0, 1, 0, 0, 0, 1, 0],
  normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
  indices: [0, 1, 2]
};

test("cold-metal Matcap is a bundled 256px WebP asset", async () => {
  const assetUrl = new URL("../ui/src/assets/cold-metal.webp", import.meta.url);
  const bytes = await readFile(assetUrl);

  assert.match(COLD_METAL_MATCAP_URL, /cold-metal\.webp$/);
  assert.equal(bytes.toString("ascii", 0, 4), "RIFF");
  assert.equal(bytes.toString("ascii", 8, 12), "WEBP");
  assert.equal(bytes.toString("ascii", 12, 16), "VP8 ");
  assert.deepEqual(readLossyWebpDimensions(bytes), { width: 256, height: 256 });
});

test("cold-metal Matcap loader uses the bundled URL and marks the texture as sRGB", async () => {
  const texture = new THREE.Texture();
  let requestedUrl: string | null = null;
  const loaded = await loadColdMetalMatcap({
    async loadAsync(url: string) {
      requestedUrl = url;
      return texture;
    }
  });

  assert.equal(requestedUrl, COLD_METAL_MATCAP_URL);
  assert.equal(loaded, texture);
  assert.equal(loaded.colorSpace, THREE.SRGBColorSpace);
});

test("STL preview uses the cold-metal Matcap material and disposes all owned resources", () => {
  const texture = new THREE.Texture();
  const object = createStlPreviewObject(triangleMesh, texture);
  const disposed = { geometry: false, material: false, texture: false };
  object.geometry.addEventListener("dispose", () => { disposed.geometry = true; });
  object.material.addEventListener("dispose", () => { disposed.material = true; });
  texture.addEventListener("dispose", () => { disposed.texture = true; });

  assert.ok(object.material instanceof THREE.MeshMatcapMaterial);
  assert.equal(object.material.color.getHex(), 0xa8bac5);
  assert.equal(object.material.matcap, texture);

  disposeObject(object);
  assert.deepEqual(disposed, { geometry: true, material: true, texture: true });
});

test("Matcap preview scene does not create Ambient or Directional lights", () => {
  const scene = createPreviewScene();
  assert.equal(scene.children.some((child) => child instanceof THREE.Light), false);
});

test("Matcap load failures are shown by the Preview error boundary", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const originalLoadAsync = THREE.TextureLoader.prototype.loadAsync;
  const originalConsoleError = console.error;
  THREE.TextureLoader.prototype.loadAsync = async () => {
    throw new Error("WebP decode failed");
  };
  console.error = () => undefined;

  try {
    await act(async () => {
      root.render(
        <UiErrorBoundary scope="Preview">
          <MeshPreview mesh={triangleMesh} gcode={null} mode="stl" />
        </UiErrorBoundary>
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    const boundary = container.querySelector("[data-testid='preview-error-boundary']");
    assert.ok(boundary);
    assert.match(boundary.textContent ?? "", /Cold-metal Matcap texture failed to load or decode/);
    assert.equal(container.querySelector("canvas"), null);
  } finally {
    THREE.TextureLoader.prototype.loadAsync = originalLoadAsync;
    console.error = originalConsoleError;
    act(() => root.unmount());
    browserWindow.close();
  }
});

test("Matcap loader rejects instead of substituting a fallback material", async () => {
  await assert.rejects(
    () => loadColdMetalMatcap({
      async loadAsync() {
        throw new Error("unsupported WebP");
      }
    }),
    /Cold-metal Matcap texture failed to load or decode/
  );
});

test("invalid STL geometry is shown by the Preview error boundary", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const invalidMesh: CadMesh = {
    vertices: [0, 0, 0, 1, 0, 0, 2, 0, 0],
    normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
    indices: [0, 1, 2]
  };
  const texture = new THREE.Texture();
  let textureDisposed = false;
  const originalLoadAsync = THREE.TextureLoader.prototype.loadAsync;
  const originalConsoleError = console.error;
  THREE.TextureLoader.prototype.loadAsync = async () => texture;
  texture.addEventListener("dispose", () => { textureDisposed = true; });
  console.error = () => undefined;

  try {
    await act(async () => {
      root.render(
        <UiErrorBoundary scope="Preview">
          <MeshPreview mesh={invalidMesh} gcode={null} mode="stl" />
        </UiErrorBoundary>
      );
      await Promise.resolve();
      await Promise.resolve();
    });

    const boundary = container.querySelector("[data-testid='preview-error-boundary']");
    assert.ok(boundary);
    assert.match(boundary.textContent ?? "", /triangle 0 is degenerate/);
    assert.equal(textureDisposed, true);
    assert.equal(container.querySelector("canvas"), null);
  } finally {
    THREE.TextureLoader.prototype.loadAsync = originalLoadAsync;
    console.error = originalConsoleError;
    act(() => root.unmount());
    browserWindow.close();
  }
});

test("an in-flight Matcap texture is disposed when the Preview unmounts", async () => {
  const browserWindow = installDom();
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const texture = new THREE.Texture();
  let textureDisposed = false;
  let resolveTexture: ((texture: THREE.Texture) => void) | null = null;
  const originalLoadAsync = THREE.TextureLoader.prototype.loadAsync;
  THREE.TextureLoader.prototype.loadAsync = () => new Promise<THREE.Texture>((resolve) => {
    resolveTexture = resolve;
  });
  texture.addEventListener("dispose", () => { textureDisposed = true; });

  try {
    await act(async () => {
      root.render(<MeshPreview mesh={triangleMesh} gcode={null} mode="stl" />);
    });
    await act(async () => {
      root.unmount();
      if (!resolveTexture) throw new Error("Matcap load did not start.");
      resolveTexture(texture);
      await Promise.resolve();
    });
    assert.equal(textureDisposed, true);
  } finally {
    THREE.TextureLoader.prototype.loadAsync = originalLoadAsync;
    browserWindow.close();
  }
});

function readLossyWebpDimensions(bytes: Buffer): { width: number; height: number } {
  if (bytes.toString("hex", 23, 26) !== "9d012a") {
    throw new Error("Cold-metal Matcap is not a valid lossy WebP bitstream.");
  }
  return {
    width: bytes.readUInt16LE(26) & 0x3fff,
    height: bytes.readUInt16LE(28) & 0x3fff
  };
}

function installDom(): Window {
  const browserWindow = new Window({ url: "http://127.0.0.1:5173/sessions/test" });
  defineGlobal("window", browserWindow);
  defineGlobal("document", browserWindow.document);
  defineGlobal("navigator", browserWindow.navigator);
  defineGlobal("HTMLElement", browserWindow.HTMLElement);
  defineGlobal("SVGElement", browserWindow.SVGElement);
  defineGlobal("Event", browserWindow.Event);
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
