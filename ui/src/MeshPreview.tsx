import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GCodeLoader } from "three/addons/loaders/GCodeLoader.js";
import type { CadMesh } from "./protocol";
import type { PreviewMode } from "./components/WorkspacePanel";
import {
  createDemandRenderScheduler,
  type DemandRenderScheduler
} from "./previewRenderScheduler";

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

  useEffect(() => {
    const container = ref.current;
    const hasPreview = mode === "stl" ? Boolean(activeMesh) : Boolean(gcodeObject);
    if (!container || !hasPreview) return;
    return mountPreview({ activeMesh, bedBounds, container, gcodeObject, mode });
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
  mode
}: {
  activeMesh: CadMesh | null;
  bedBounds: RectangularBedShape | null;
  container: HTMLDivElement;
  gcodeObject: THREE.Group | null;
  mode: PreviewMode;
}, runtime: PreviewRuntime = createBrowserPreviewRuntime()): () => void {
  const width = Math.max(container.clientWidth, 1);
  const height = Math.max(container.clientHeight, 1);
  const scene = new THREE.Scene();
  scene.background = new THREE.Color(0xf6f7f9);
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
    if (axisLines) disposeObject(axisLines);
    if (bedGrid) disposeObject(bedGrid);
    renderer?.dispose();
    if (renderer?.domElement.parentNode === container) {
      container.removeChild(renderer.domElement);
    }
  };

  try {
    renderer = runtime.createRenderer();
    renderer.setPixelRatio(Math.min(runtime.devicePixelRatio, PREVIEW_PIXEL_RATIO_LIMIT));
    renderer.setSize(width, height, false);
    container.appendChild(renderer.domElement);

    const ambient = new THREE.AmbientLight(0xffffff, 1.8);
    const key = new THREE.DirectionalLight(0xffffff, 2.4);
    key.position.set(50, -80, 120);
    scene.add(ambient, key);
    axisLines = mode === "stl" ? createAxisLines(5000) : null;
    if (axisLines) scene.add(axisLines);
    bedGrid = mode === "gcode" && bedBounds ? createBedGrid(bedBounds) : null;
    if (bedGrid) scene.add(bedGrid);

    controls = runtime.createControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.enableRotate = true;
    controls.enableZoom = true;
    controls.enablePan = true;
    controls.dampingFactor = 0.08;
    controls.zoomSpeed = 0.9;
    controls.rotateSpeed = 0.7;

    if (mode === "stl") {
      if (!activeMesh) throw new Error("STL preview mesh is unavailable.");
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.Float32BufferAttribute(activeMesh.vertices, 3));
      geometry.setAttribute("normal", new THREE.Float32BufferAttribute(activeMesh.normals, 3));
      geometry.setIndex(activeMesh.indices);
      geometry.computeBoundingBox();
      geometry.computeBoundingSphere();
      const material = new THREE.MeshStandardMaterial({
        color: 0x2c7a7b,
        metalness: 0.08,
        roughness: 0.48
      });
      modelObject = new THREE.Mesh(geometry, material);
    } else {
      if (!gcodeObject) throw new Error("G-code preview is unavailable.");
      modelObject = gcodeObject;
    }
    scene.add(modelObject);

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

function disposeObject(object: THREE.Object3D): void {
  const geometries = new Set<THREE.BufferGeometry>();
  const materials = new Set<THREE.Material>();
  object.traverse((child) => {
    if ("geometry" in child && child.geometry instanceof THREE.BufferGeometry) {
      geometries.add(child.geometry);
    }
    if (!("material" in child)) return;
    const childMaterials = Array.isArray(child.material) ? child.material : [child.material];
    for (const material of childMaterials) {
      if (material instanceof THREE.Material) materials.add(material);
    }
  });
  for (const geometry of geometries) geometry.dispose();
  for (const material of materials) material.dispose();
}
