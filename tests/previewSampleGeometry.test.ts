import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import * as THREE from "three";
import { parseStlToMesh } from "../ui/src/runtime/stl";
import {
  STL_CREASE_ANGLE_RADIANS,
  createStlPreviewGeometry,
  getStlPreviewGeometryStats
} from "../ui/src/stlPreviewGeometry";

const NORMAL_EPSILON = 1e-3;

test("representative curved and sharp-edge STL samples retain readable normal transitions", async () => {
  const samples = ["shaft", "pulley", "bracket"];

  for (const sample of samples) {
    const bytes = await readFile(new URL(`../sample/part/${sample}.stl`, import.meta.url));
    const mesh = parseStlToMesh(bytes);
    const geometry = createStlPreviewGeometry(mesh);
    const stats = getStlPreviewGeometryStats(geometry);
    const positions = geometry.getAttribute("position");
    const normals = geometry.getAttribute("normal");

    assert.equal(stats.triangleCount, mesh.indices.length / 3, sample);
    assert.ok(stats.mergedVertexCount < stats.sourceVertexCount, sample);
    assert.equal(positions.count, mesh.indices.length, sample);
    assert.equal(normals.count, positions.count, sample);
    for (let index = 0; index < normals.count; index += 1) {
      const normal = new THREE.Vector3().fromBufferAttribute(normals, index);
      assert.ok(isApproximately(normal.length(), 1, NORMAL_EPSILON), `${sample} normal ${index}`);
    }

    const transitions = findNormalTransitions(mesh.normals, positions, normals);
    assert.equal(transitions.hasSmoothed, true, `${sample} must smooth at least one faceted join`);
    assert.equal(transitions.hasSharp, true, `${sample} must preserve at least one sharp edge`);
    assert.ok(geometry.boundingBox && !geometry.boundingBox.isEmpty(), sample);
    assert.ok((geometry.boundingSphere?.radius ?? 0) > 0, sample);
    geometry.dispose();
  }
});

function findNormalTransitions(
  sourceNormals: number[],
  positions: THREE.BufferAttribute | THREE.InterleavedBufferAttribute,
  derivedNormals: THREE.BufferAttribute | THREE.InterleavedBufferAttribute
): { hasSmoothed: boolean; hasSharp: boolean } {
  const cornersByPosition = new Map<string, number[]>();
  for (let index = 0; index < positions.count; index += 1) {
    const key = `${positions.getX(index)},${positions.getY(index)},${positions.getZ(index)}`;
    const corners = cornersByPosition.get(key) ?? [];
    corners.push(index);
    cornersByPosition.set(key, corners);
  }

  const creaseDot = Math.cos(STL_CREASE_ANGLE_RADIANS);
  let hasSmoothed = false;
  let hasSharp = false;
  for (const corners of cornersByPosition.values()) {
    for (let left = 0; left < corners.length; left += 1) {
      for (let right = left + 1; right < corners.length; right += 1) {
        const leftIndex = corners[left];
        const rightIndex = corners[right];
        const sourceDot = dotArrayNormals(sourceNormals, leftIndex, rightIndex);
        const derivedDot = new THREE.Vector3()
          .fromBufferAttribute(derivedNormals, leftIndex)
          .dot(new THREE.Vector3().fromBufferAttribute(derivedNormals, rightIndex));
        if (
          !isApproximately(sourceDot, 1, NORMAL_EPSILON)
          && isApproximately(derivedDot, 1, NORMAL_EPSILON)
        ) {
          hasSmoothed = true;
        }
        if (derivedDot <= creaseDot) hasSharp = true;
        if (hasSmoothed && hasSharp) return { hasSmoothed, hasSharp };
      }
    }
  }
  return { hasSmoothed, hasSharp };
}

function isApproximately(value: number, expected: number, epsilon: number): boolean {
  return Math.abs(value - expected) <= epsilon;
}

function dotArrayNormals(normals: number[], left: number, right: number): number {
  return (
    normals[left * 3] * normals[right * 3]
    + normals[left * 3 + 1] * normals[right * 3 + 1]
    + normals[left * 3 + 2] * normals[right * 3 + 2]
  );
}
