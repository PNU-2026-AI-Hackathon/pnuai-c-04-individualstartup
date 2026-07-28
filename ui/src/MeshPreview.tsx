import { useEffect, useRef } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import type { CadMesh } from "./protocol";

export function MeshPreview({ mesh }: { mesh: CadMesh | null }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = ref.current;
    if (!container || !mesh) return;
    const width = Math.max(container.clientWidth, 1);
    const height = Math.max(container.clientHeight, 1);
    const scene = new THREE.Scene();
    scene.background = new THREE.Color(0xf6f7f9);

    const camera = new THREE.PerspectiveCamera(35, width / height, 0.1, 5000);

    const renderer = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
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

    let modelGeometry: THREE.BufferGeometry | undefined;
    let modelMaterial: THREE.Material | undefined;
    if (mesh) {
      const geometry = new THREE.BufferGeometry();
      geometry.setAttribute("position", new THREE.Float32BufferAttribute(mesh.vertices, 3));
      geometry.setAttribute("normal", new THREE.Float32BufferAttribute(mesh.normals, 3));
      geometry.setIndex(mesh.indices);
      geometry.computeBoundingBox();
      geometry.computeBoundingSphere();
      modelGeometry = geometry;
      modelMaterial = new THREE.MeshStandardMaterial({
        color: 0x2c7a7b,
        metalness: 0.08,
        roughness: 0.48
      });
      const model = new THREE.Mesh(geometry, modelMaterial);
      scene.add(model);
    }

    const bounds = modelGeometry?.boundingSphere;
    const center = bounds?.center ?? new THREE.Vector3(0, 0, 0);
    const radius = Math.max(bounds?.radius ?? 70, 1);
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
      modelGeometry?.dispose();
      modelMaterial?.dispose();
      renderer.dispose();
      container.removeChild(renderer.domElement);
    };
  }, [mesh]);

  return <div className="mesh-preview" ref={ref}>{!mesh ? <span>No preview rendered</span> : null}</div>;
}
