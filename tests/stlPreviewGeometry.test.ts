import assert from "node:assert/strict";
import test from "node:test";
import * as THREE from "three";
import {
  STL_CREASE_ANGLE_RADIANS,
  createStlPreviewGeometry,
  getStlPreviewGeometryStats
} from "../ui/src/stlPreviewGeometry";
import type { CadMesh } from "../ui/src/protocol";

test("creased normals smooth a shallow curved join after scale-relative vertex merging", () => {
  const foldAngle = THREE.MathUtils.degToRad(20);
  const mesh = createFoldMesh(foldAngle, 1e-7);
  const original = structuredClone(mesh);
  const geometry = createStlPreviewGeometry(mesh);
  const normals = geometry.getAttribute("normal");
  const stats = getStlPreviewGeometryStats(geometry);

  assert.deepEqual(mesh, original);
  assert.equal(geometry.index, null);
  assert.equal(geometry.getAttribute("position").count, 6);
  assert.equal(stats.sourceVertexCount, 6);
  assert.equal(stats.mergedVertexCount, 4);
  assert.equal(stats.triangleCount, 2);
  assert.equal(stats.creaseAngleRadians, STL_CREASE_ANGLE_RADIANS);
  assert.ok(stats.vertexMergeTolerance > 1e-7);

  const firstSharedNormal = new THREE.Vector3().fromBufferAttribute(normals, 0);
  const secondSharedNormal = new THREE.Vector3().fromBufferAttribute(normals, 3);
  assert.ok(firstSharedNormal.distanceTo(secondSharedNormal) < 1e-6);
  assert.ok(Math.abs(firstSharedNormal.y + Math.sin(foldAngle / 2)) < 1e-6);
  assert.ok(Math.abs(firstSharedNormal.z - Math.cos(foldAngle / 2)) < 1e-6);
  geometry.dispose();
});

test("creased normals retain 90-degree cube edges and final geometry bounds", () => {
  const source = new THREE.BoxGeometry(1, 1, 1);
  const mesh = geometryToCadMesh(source);
  const geometry = createStlPreviewGeometry(mesh);
  const positions = geometry.getAttribute("position");
  const normals = geometry.getAttribute("normal");
  const cornerNormals: THREE.Vector3[] = [];

  for (let index = 0; index < positions.count; index += 1) {
    const position = new THREE.Vector3().fromBufferAttribute(positions, index);
    if (position.distanceTo(new THREE.Vector3(0.5, 0.5, 0.5)) < 1e-6) {
      cornerNormals.push(new THREE.Vector3().fromBufferAttribute(normals, index));
    }
  }

  const stats = getStlPreviewGeometryStats(geometry);
  assert.equal(stats.sourceVertexCount, 24);
  assert.equal(stats.mergedVertexCount, 8);
  assert.equal(stats.triangleCount, 12);
  assert.equal(geometry.getAttribute("position").count, 36);
  assert.deepEqual(geometry.boundingBox?.min.toArray(), [-0.5, -0.5, -0.5]);
  assert.deepEqual(geometry.boundingBox?.max.toArray(), [0.5, 0.5, 0.5]);
  assert.ok(Math.abs((geometry.boundingSphere?.radius ?? 0) - Math.sqrt(3) / 2) < 1e-6);
  assert.equal(cornerNormals.length, 4);
  assert.ok(cornerNormals.every((normal) => (
    [Math.abs(normal.x), Math.abs(normal.y), Math.abs(normal.z)].filter((value) => value > 0.999).length === 1
  )));
  assert.equal(new Set(cornerNormals.map((normal) => normal.toArray().join(","))).size, 3);

  source.dispose();
  geometry.dispose();
});

test("vertex merge tolerance follows model scale without joining a distinct feature", () => {
  const foldAngle = THREE.MathUtils.degToRad(20);
  const unitGeometry = createStlPreviewGeometry(createFoldMesh(foldAngle, 1e-7));
  const largeMesh = createFoldMesh(foldAngle, 1e-7);
  largeMesh.vertices = largeMesh.vertices.map((coordinate) => coordinate * 1e6);
  const largeGeometry = createStlPreviewGeometry(largeMesh);
  const distinctGeometry = createStlPreviewGeometry(createFoldMesh(foldAngle, 1e-4));
  const unitStats = getStlPreviewGeometryStats(unitGeometry);
  const largeStats = getStlPreviewGeometryStats(largeGeometry);
  const distinctStats = getStlPreviewGeometryStats(distinctGeometry);

  assert.ok(Math.abs(largeStats.vertexMergeTolerance / unitStats.vertexMergeTolerance - 1e6) < 1);
  assert.equal(unitStats.mergedVertexCount, 4);
  assert.equal(largeStats.mergedVertexCount, 4);
  assert.equal(distinctStats.mergedVertexCount, 6);
  unitGeometry.dispose();
  largeGeometry.dispose();
  distinctGeometry.dispose();
});

test("merge tolerance preserves distinct vertices that form a small triangle feature", () => {
  const geometry = createStlPreviewGeometry({
    vertices: [0, 0, 0, 5e-7, 0, 0, 0, 1, 0],
    normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
    indices: [0, 1, 2]
  });

  assert.equal(getStlPreviewGeometryStats(geometry).mergedVertexCount, 3);
  assert.equal(geometry.getAttribute("position").count, 3);
  geometry.dispose();
});

test("derived geometry disposal is observable for every repeated preview instance", () => {
  const first = createStlPreviewGeometry(createFoldMesh(0, 0));
  const second = createStlPreviewGeometry(createFoldMesh(0, 0));
  let disposeCount = 0;
  first.addEventListener("dispose", () => { disposeCount += 1; });
  second.addEventListener("dispose", () => { disposeCount += 1; });

  first.dispose();
  second.dispose();
  assert.equal(disposeCount, 2);
});

test("invalid and degenerate STL geometry fails explicitly", () => {
  const valid = createFoldMesh(0, 0);
  const nonFiniteVertices = [...valid.vertices];
  nonFiniteVertices[0] = Number.NaN;
  assert.throws(
    () => createStlPreviewGeometry({ ...valid, vertices: nonFiniteVertices }),
    /coordinate 0 is not finite/
  );
  assert.throws(
    () => createStlPreviewGeometry({ ...valid, indices: [0, 1, 99] }),
    /references an invalid vertex/
  );
  assert.throws(
    () => createStlPreviewGeometry({
      vertices: [0, 0, 0, 1, 0, 0, 2, 0, 0],
      normals: [0, 0, 1, 0, 0, 1, 0, 0, 1],
      indices: [0, 1, 2]
    }),
    /degenerate/
  );
});

function createFoldMesh(angle: number, duplicateOffset: number): CadMesh {
  const cosine = Math.cos(angle);
  const sine = Math.sin(angle);
  return {
    vertices: [
      0, 0, 0,
      1, 0, 0,
      0, 1, 0,
      duplicateOffset, 0, 0,
      1 + duplicateOffset, 0, 0,
      0, cosine, sine
    ],
    normals: Array.from({ length: 18 }, () => 0),
    indices: [0, 1, 2, 3, 4, 5]
  };
}

function geometryToCadMesh(geometry: THREE.BufferGeometry): CadMesh {
  const positions = geometry.getAttribute("position");
  const normals = geometry.getAttribute("normal");
  const index = geometry.getIndex();
  return {
    vertices: Array.from(positions.array),
    normals: Array.from(normals.array),
    indices: index ? Array.from(index.array) : Array.from({ length: positions.count }, (_, item) => item)
  };
}
