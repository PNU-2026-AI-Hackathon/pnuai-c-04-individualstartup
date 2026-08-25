import { useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GCodeLoader } from "three/addons/loaders/GCodeLoader.js";
import type { CadMesh } from "./protocol";
import type { PreviewMode } from "./components/WorkspacePanel";
import {
  createDemandRenderScheduler,
  type DemandRenderScheduler
} from "./previewRenderScheduler";
import { createStlPreviewGeometry } from "./stlPreviewGeometry";

// 1.5 keeps diagonal edges crisp while limiting fragment work on high-DPR displays.
export const PREVIEW_PIXEL_RATIO_LIMIT = 1.5;

type PreviewRenderer = Pick<
  THREE.WebGLRenderer,
  "domElement" | "setPixelRatio" | "setSize" | "render" | "dispose"
>;

type PreviewControls = Pick<
  OrbitControls,
  | "target"
  | "minDistance"
  | "maxDistance"
  | "enableDamping"
  | "enableRotate"
  | "enableZoom"
  | "enablePan"
  | "dampingFactor"
  | "zoomSpeed"
  | "rotateSpeed"
  | "update"
  | "getDistance"
  | "addEventListener"
  | "removeEventListener"
  | "dispose"
>;

export interface PreviewRuntime {
  devicePixelRatio: number;
  createRenderer(): PreviewRenderer;
  createControls(camera: THREE.PerspectiveCamera, element: HTMLElement): PreviewControls;
  createResizeObserver(callback: ResizeObserverCallback): Pick<ResizeObserver, "observe" | "disconnect">;
  requestAnimationFrame(callback: FrameRequestCallback): number;
  cancelAnimationFrame(handle: number): void;
}

function createBrowserPreviewRuntime(): PreviewRuntime {
  return {
    devicePixelRatio: window.devicePixelRatio,
    createRenderer: () => new THREE.WebGLRenderer({ antialias: true }),
    createControls: (camera, element) => new OrbitControls(camera, element),
    createResizeObserver: (callback) => new ResizeObserver(callback),
    requestAnimationFrame: (callback) => window.requestAnimationFrame(callback),
    cancelAnimationFrame: (handle) => window.cancelAnimationFrame(handle)
  };
}

export const COLD_METAL_MATCAP_URL = new URL("./assets/cold-metal.webp", import.meta.url).href;

type MatcapTextureLoader = Pick<THREE.TextureLoader, "loadAsync">;

export async function loadColdMetalMatcap(
  loader: MatcapTextureLoader = new THREE.TextureLoader()
): Promise<THREE.Texture> {
  try {
    const texture = await loader.loadAsync(COLD_METAL_MATCAP_URL);
    texture.colorSpace = THREE.SRGBColorSpace;
    return texture;
  } catch (cause) {
    throw new Error("Cold-metal Matcap texture failed to load or decode.", { cause });
  }
}

export function MeshPreview({
  mesh,
  gcode,
  bedShape,
  mode
}: {
  mesh: CadMesh | null;
  gcode: string | null;
  bedShape?: unknown;
  mode: PreviewMode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const gcodeObject = useMemo(
    () => mode === "gcode" && gcode ? parseRenderableGCode(gcode) : null,
    [gcode, mode]
  );
  const bedBounds = useMemo(
    () => mode === "gcode" && bedShape !== undefined ? parseRectangularBedShape(bedShape) : null,
    [bedShape, mode]
  );
  const activeMesh = mode === "stl" ? mesh : null;
  const [previewFailure, setPreviewFailure] = useState<{
    mode: PreviewMode;
    mesh: CadMesh | null;
    error: Error;
  } | null>(null);

  if (
    previewFailure
    && previewFailure.mode === mode
    && previewFailure.mesh === activeMesh
  ) {
    throw previewFailure.error;
  }

  useEffect(() => {
    const container = ref.current;
    const hasPreview = mode === "stl" ? Boolean(activeMesh) : Boolean(gcodeObject);
    if (!container || !hasPreview) return;
    let cancelled = false;
    let disposePreview: (() => void) | null = null;

    const initialize = async () => {
      const matcap = mode === "stl" ? await loadColdMetalMatcap() : null;
      if (cancelled) {
        matcap?.dispose();
        return;
      }
      disposePreview = mountPreview({
        activeMesh,
        bedBounds,
        container,
        gcodeObject,
        matcap,
        mode
      });
    };

    void initialize().catch((caught: unknown) => {
      if (cancelled) return;
      const error = caught instanceof Error
        ? caught
        : new Error("Preview initialization failed with an unknown error.", { cause: caught });
      setPreviewFailure({ mode, mesh: activeMesh, error });
    });

    return () => {
      cancelled = true;
      disposePreview?.();
    };
  }, [activeMesh, bedBounds, gcodeObject, mode]);

  const hasPreview = mode === "stl" ? Boolean(mesh) : Boolean(gcodeObject);
  return (
    <div className="mesh-preview" data-preview-mode={mode} ref={ref}>
      {!hasPreview ? <span>No {mode === "stl" ? "STL" : "G-code"} preview available</span> : null}
    </div>
  );
}

export function mountPreview({
  activeMesh,
  bedBounds,
  container,
  gcodeObject,
  matcap,
  mode
}: {
  activeMesh: CadMesh | null;
  bedBounds: RectangularBedShape | null;
  container: HTMLDivElement;
  gcodeObject: THREE.Group | null;
  matcap: THREE.Texture | null;
  mode: PreviewMode;
}, runtime: PreviewRuntime = createBrowserPreviewRuntime()): () => void {
  const width = Math.max(container.clientWidth, 1);
  const height = Math.max(container.clientHeight, 1);
  const scene = createPreviewScene();
  const camera = new THREE.PerspectiveCamera(35, width / height, 0.1, 5000);
  let renderer: PreviewRenderer | null = null;
  let controls: PreviewControls | null = null;
  let observer: Pick<ResizeObserver, "observe" | "disconnect"> | null = null;
  let modelObject: THREE.Object3D | null = null;
  let axisLines: THREE.LineSegments | null = null;
  let bedGrid: THREE.LineSegments | null = null;
  let scheduler: DemandRenderScheduler | null = null;
  let updateCameraDebugState: (() => void) | null = null;
  let handleControlsChange: (() => void) | null = null;
  let ownsUnattachedMatcap = matcap !== null;
  let disposed = false;

  const dispose = () => {
    if (disposed) return;
    disposed = true;
    scheduler?.dispose();
    observer?.disconnect();
    if (controls && handleControlsChange) {
      controls.removeEventListener("change", handleControlsChange);
    }
    controls?.dispose();
    if (modelObject) disposeObject(modelObject);
    if (ownsUnattachedMatcap) matcap?.dispose();
    if (axisLines) disposeObject(axisLines);
    if (bedGrid) disposeObject(bedGrid);
    renderer?.dispose();
    if (renderer?.domElement.parentNode === container) {
      container.removeChild(renderer.domElement);
    }
  };

  try {
    axisLines = mode === "stl" ? createAxisLines(5000) : null;
    if (axisLines) scene.add(axisLines);
    bedGrid = mode === "gcode" && bedBounds ? createBedGrid(bedBounds) : null;
    if (bedGrid) scene.add(bedGrid);

    if (mode === "stl") {
      if (!activeMesh) throw new Error("STL preview mesh is unavailable.");
      if (!matcap) throw new Error("Cold-metal Matcap texture is unavailable.");
      modelObject = createStlPreviewObject(activeMesh, matcap);
      ownsUnattachedMatcap = false;
    } else {
      if (!gcodeObject) throw new Error("G-code preview is unavailable.");
      modelObject = gcodeObject;
    }
    scene.add(modelObject);

    renderer = runtime.createRenderer();
    renderer.setPixelRatio(Math.min(runtime.devicePixelRatio, PREVIEW_PIXEL_RATIO_LIMIT));
    renderer.setSize(width, height, false);
    container.appendChild(renderer.domElement);

    controls = runtime.createControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.enableRotate = true;
    controls.enableZoom = true;
    controls.enablePan = true;
    controls.dampingFactor = 0.08;
    controls.zoomSpeed = 0.9;
    controls.rotateSpeed = 0.7;

    const bounds = new THREE.Box3().setFromObject(modelObject);
    if (bedGrid) bounds.expandByObject(bedGrid);
    const sphere = bounds.getBoundingSphere(new THREE.Sphere());
    const center = sphere.center;
    const radius = Math.max(sphere.radius, 1);
    controls.target.copy(center);
    controls.minDistance = radius * 0.25;
    controls.maxDistance = radius * 12;
    camera.near = Math.max(radius / 200, 0.1);
    camera.far = radius * 40;
    camera.position.copy(center).add(new THREE.Vector3(radius * 1.35, -radius * 1.75, radius * 1.35));
    camera.updateProjectionMatrix();
    controls.update();

    updateCameraDebugState = () => {
      if (!controls) return;
      container.dataset.cameraDistance = controls.getDistance().toFixed(3);
      container.dataset.cameraPosition = [camera.position.x, camera.position.y, camera.position.z]
        .map((value) => value.toFixed(3))
        .join(",");
    };
    updateCameraDebugState();

    scheduler = createDemandRenderScheduler({
      update: () => controls?.update() ?? false,
      render: () => {
        renderer?.render(scene, camera);
        updateCameraDebugState?.();
      },
      requestAnimationFrame: runtime.requestAnimationFrame,
      cancelAnimationFrame: runtime.cancelAnimationFrame
    });
    handleControlsChange = () => {
      if (disposed) return;
      updateCameraDebugState?.();
      scheduler?.requestRender();
    };
    controls.addEventListener("change", handleControlsChange);

    let renderedWidth = width;
    let renderedHeight = height;
    const resize = () => {
      if (disposed || !renderer) return;
      const nextWidth = Math.max(container.clientWidth, 1);
      const nextHeight = Math.max(container.clientHeight, 1);
      if (nextWidth === renderedWidth && nextHeight === renderedHeight) return;
      renderedWidth = nextWidth;
      renderedHeight = nextHeight;
      camera.aspect = nextWidth / nextHeight;
      camera.updateProjectionMatrix();
      renderer.setSize(nextWidth, nextHeight, false);
      scheduler?.requestRender();
    };
    observer = runtime.createResizeObserver(resize);
    observer.observe(container);

    scheduler.requestRender();
    return dispose;
  } catch (error) {
    dispose();
    throw error;
  }
}

export function createStlPreviewObject(
  mesh: CadMesh,
  matcap: THREE.Texture
): THREE.Mesh<THREE.BufferGeometry, THREE.MeshMatcapMaterial> {
  const geometry = createStlPreviewGeometry(mesh);
  const material = new THREE.MeshMatcapMaterial({
    color: 0xa8bac5,
    matcap
  });
  return new THREE.Mesh(geometry, material);
}

export function createPreviewScene(): THREE.Scene {
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0xf6f7f9);
  return scene;
}

function createAxisLines(extent: number): THREE.LineSegments {
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute([
    -extent, 0, 0, extent, 0, 0,
    0, -extent, 0, 0, extent, 0,
    0, 0, -extent, 0, 0, extent
  ], 3));
  geometry.setAttribute("color", new THREE.Float32BufferAttribute([
    1, 0, 0, 1, 0, 0,
    0, 1, 0, 0, 1, 0,
    0, 0, 1, 0, 0, 1
  ], 3));
  return new THREE.LineSegments(
    geometry,
    new THREE.LineBasicMaterial({ vertexColors: true, toneMapped: false })
  );
}

export type RectangularBedShape = {
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
};

export function parseRectangularBedShape(value: unknown): RectangularBedShape {
  if (!Array.isArray(value) || value.length !== 4) {
    throw new Error("G-code bedShape metadata must contain four corner points.");
  }
  const points = value.map((point, index) => {
    if (
      !Array.isArray(point) ||
      point.length !== 2 ||
      typeof point[0] !== "number" ||
      typeof point[1] !== "number" ||
      !Number.isFinite(point[0]) ||
      !Number.isFinite(point[1])
    ) {
      throw new Error(`G-code bedShape point ${index + 1} must contain two finite coordinates.`);
    }
    return { x: point[0], y: point[1] };
  });
  const minX = Math.min(...points.map((point) => point.x));
  const maxX = Math.max(...points.map((point) => point.x));
  const minY = Math.min(...points.map((point) => point.y));
  const maxY = Math.max(...points.map((point) => point.y));
  if (minX === maxX || minY === maxY) {
    throw new Error("G-code bedShape must have non-zero width and depth.");
  }
  const expectedCorners = new Set([
    `${minX}:${minY}`,
    `${maxX}:${minY}`,
    `${maxX}:${maxY}`,
    `${minX}:${maxY}`
  ]);
  for (const point of points) expectedCorners.delete(`${point.x}:${point.y}`);
  if (expectedCorners.size !== 0) {
    throw new Error("G-code bedShape must describe an axis-aligned rectangular bed.");
  }
  return { minX, maxX, minY, maxY };
}

export function createBedGrid(
  bounds: RectangularBedShape,
  spacing = 10
): THREE.LineSegments {
  if (!Number.isFinite(spacing) || spacing <= 0) {
    throw new Error("G-code bed grid spacing must be greater than zero.");
  }
  const positions: number[] = [];
  const colors: number[] = [];
  const addSegment = (x1: number, z1: number, x2: number, z2: number, color: [number, number, number]) => {
    positions.push(x1, 0, z1, x2, 0, z2);
    colors.push(...color, ...color);
  };
  const gridColor: [number, number, number] = [0.72, 0.76, 0.8];
  const edgeColor: [number, number, number] = [0.38, 0.44, 0.5];
  for (let x = Math.ceil(bounds.minX / spacing) * spacing; x < bounds.maxX; x += spacing) {
    if (x > bounds.minX) addSegment(x, -bounds.minY, x, -bounds.maxY, gridColor);
  }
  for (let y = Math.ceil(bounds.minY / spacing) * spacing; y < bounds.maxY; y += spacing) {
    if (y > bounds.minY) addSegment(bounds.minX, -y, bounds.maxX, -y, gridColor);
  }
  addSegment(bounds.minX, -bounds.minY, bounds.maxX, -bounds.minY, edgeColor);
  addSegment(bounds.maxX, -bounds.minY, bounds.maxX, -bounds.maxY, edgeColor);
  addSegment(bounds.maxX, -bounds.maxY, bounds.minX, -bounds.maxY, edgeColor);
  addSegment(bounds.minX, -bounds.maxY, bounds.minX, -bounds.minY, edgeColor);
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("color", new THREE.Float32BufferAttribute(colors, 3));
  return new THREE.LineSegments(
    geometry,
    new THREE.LineBasicMaterial({ vertexColors: true, transparent: true, opacity: 0.8, toneMapped: false })
  );
}

export function parseRenderableGCode(gcode: string): THREE.Group {
  if (!gcode.trim()) throw new Error("G-code preview input is empty.");
  const object = new GCodeLoader().parse(gcode);
  let vertexCount = 0;
  let invalidCoordinate = false;
  object.traverse((child) => {
    if (!(child instanceof THREE.LineSegments)) return;
    const positions = child.geometry.getAttribute("position");
    vertexCount += positions?.count ?? 0;
    if (positions) {
      for (let index = 0; index < positions.count * positions.itemSize; index += 1) {
        if (!Number.isFinite(positions.array[index])) invalidCoordinate = true;
      }
    }
    const materials = Array.isArray(child.material) ? child.material : [child.material];
    for (const material of materials) {
      if (material instanceof THREE.LineBasicMaterial) {
        material.color.set(material.name === "extruded" ? 0x138a72 : 0xe06c3b);
      }
    }
  });
  if (vertexCount === 0) {
    disposeObject(object);
    throw new Error("G-code contains no renderable G0/G1 toolpath moves.");
  }
  if (invalidCoordinate) {
    disposeObject(object);
    throw new Error("G-code contains an invalid toolpath coordinate.");
  }
  return object;
}

export function disposeObject(object: THREE.Object3D): void {
  const geometries = new Set<THREE.BufferGeometry>();
  const materials = new Set<THREE.Material>();
  const textures = new Set<THREE.Texture>();
  object.traverse((child) => {
    if ("geometry" in child && child.geometry instanceof THREE.BufferGeometry) {
      geometries.add(child.geometry);
    }
    if (!("material" in child)) return;
    const childMaterials = Array.isArray(child.material) ? child.material : [child.material];
    for (const material of childMaterials) {
      if (!(material instanceof THREE.Material)) continue;
      materials.add(material);
      for (const value of Object.values(material)) {
        if (
          typeof value === "object"
          && value !== null
          && (value as THREE.Texture).isTexture === true
        ) {
          textures.add(value as THREE.Texture);
        }
      }
    }
  });
  for (const geometry of geometries) geometry.dispose();
  for (const texture of textures) texture.dispose();
  for (const material of materials) material.dispose();
}
