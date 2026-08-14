import { useEffect, useMemo, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { GCodeLoader } from "three/addons/loaders/GCodeLoader.js";
import type { CadMesh } from "./protocol";
import type { PreviewMode } from "./components/WorkspacePanel";

export function MeshPreview({
  mesh,
  gcode,
  mode
}: {
  mesh: CadMesh | null;
  gcode: string | null;
  mode: PreviewMode;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const gcodeObject = useMemo(
    () => mode === "gcode" && gcode ? parseRenderableGCode(gcode) : null,
    [gcode, mode]
  );
  const activeMesh = mode === "stl" ? mesh : null;

  useEffect(() => {
    const container = ref.current;
    const hasPreview = mode === "stl" ? Boolean(activeMesh) : Boolean(gcodeObject);
    if (!container || !hasPreview) return;
    const width = Math.max(container.clientWidth, 1);
    const height = Math.max(container.clientHeight, 1);
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0xf6f7f9);

    const camera = new THREE.PerspectiveCamera(35, width / height, 0.1, 5000);

    const renderer = new THREE.WebGLRenderer({ antialias: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setSize(width, height, false);
    container.appendChild(renderer.domElement);

    const ambient = new THREE.AmbientLight(0xffffff, 1.8);
    const key = new THREE.DirectionalLight(0xffffff, 2.4);
    key.position.set(50, -80, 120);
    scene.add(ambient, key, new THREE.GridHelper(120, 12, 0x8d99a8, 0xd8dee8));

    const controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.enableRotate = true;
    controls.enableZoom = true;
    controls.enablePan = true;
    controls.dampingFactor = 0.08;
    controls.zoomSpeed = 0.9;
    controls.rotateSpeed = 0.7;

    let modelObject: THREE.Object3D;
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

    const updateCameraDebugState = () => {
      container.dataset.cameraDistance = controls.getDistance().toFixed(3);
      container.dataset.cameraPosition = [camera.position.x, camera.position.y, camera.position.z]
        .map((value) => value.toFixed(3))
        .join(",");
    };
    controls.addEventListener("change", updateCameraDebugState);
    updateCameraDebugState();

    const resize = () => {
      const nextWidth = Math.max(container.clientWidth, 1);
      const nextHeight = Math.max(container.clientHeight, 1);
      camera.aspect = nextWidth / nextHeight;
      camera.updateProjectionMatrix();
      renderer.setSize(nextWidth, nextHeight, false);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(container);

    let frame = 0;
    const animate = () => {
      frame = requestAnimationFrame(animate);
      controls.update();
      renderer.render(scene, camera);
      updateCameraDebugState();
    };
    animate();

    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      controls.removeEventListener("change", updateCameraDebugState);
      controls.dispose();
      disposeObject(modelObject);
      renderer.dispose();
      container.removeChild(renderer.domElement);
    };
  }, [activeMesh, gcodeObject, mode]);

  const hasPreview = mode === "stl" ? Boolean(mesh) : Boolean(gcodeObject);
  return (
    <div className="mesh-preview" data-preview-mode={mode} ref={ref}>
      {!hasPreview ? <span>No {mode === "stl" ? "STL" : "G-code"} preview available</span> : null}
    </div>
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
