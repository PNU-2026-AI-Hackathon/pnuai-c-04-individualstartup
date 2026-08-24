import * as THREE from "three";
import type { CadMesh } from "./protocol";

export const STL_CREASE_ANGLE_RADIANS = THREE.MathUtils.degToRad(40);
export const STL_VERTEX_MERGE_RELATIVE_TOLERANCE = 1e-6;

export interface StlPreviewGeometryStats {
  sourceVertexCount: number;
  mergedVertexCount: number;
  triangleCount: number;
  vertexMergeTolerance: number;
  creaseAngleRadians: number;
}

type WeldedVertex = {
  position: THREE.Vector3;
  faceNormals: THREE.Vector3[];
  sourceIndices: number[];
};

/**
 * Builds non-indexed render geometry so each triangle corner can carry either a
 * smoothed normal or its own sharp-edge normal. The merge tolerance is one
 * millionth of the model diagonal: scale-relative, but far below visible CAD
 * features at ordinary model sizes.
 */
export function createStlPreviewGeometry(mesh: CadMesh): THREE.BufferGeometry {
  validateMeshArrays(mesh);

  const sourceVertexCount = mesh.vertices.length / 3;
  const sourcePositions = Array.from({ length: sourceVertexCount }, (_, index) => (
    new THREE.Vector3(
      mesh.vertices[index * 3],
      mesh.vertices[index * 3 + 1],
      mesh.vertices[index * 3 + 2]
    )
  ));
  const sourceBounds = new THREE.Box3().setFromPoints(sourcePositions);
  const diagonal = sourceBounds.min.distanceTo(sourceBounds.max);
  if (!Number.isFinite(diagonal) || diagonal <= 0) {
    throw new Error("STL preview geometry must span a non-zero finite volume or area.");
  }

  const maxCoordinate = sourcePositions.reduce(
    (maximum, position) => Math.max(maximum, Math.abs(position.x), Math.abs(position.y), Math.abs(position.z)),
    0
  );
  const vertexMergeTolerance = Math.max(
    diagonal * STL_VERTEX_MERGE_RELATIVE_TOLERANCE,
    maxCoordinate * Number.EPSILON * 16,
    Number.EPSILON
  );
  const trianglePeers = buildTrianglePeers(mesh.indices, sourceVertexCount);
  const { sourceToWelded, weldedVertices } = weldVertices(
    sourcePositions,
    trianglePeers,
    vertexMergeTolerance
  );
  const faceNormals: THREE.Vector3[] = [];
  const minimumDoubleArea = diagonal * diagonal * Number.EPSILON * 16;

  for (let index = 0; index < mesh.indices.length; index += 3) {
    const weldedIndices = [
      sourceToWelded[mesh.indices[index]],
      sourceToWelded[mesh.indices[index + 1]],
      sourceToWelded[mesh.indices[index + 2]]
    ];
    if (new Set(weldedIndices).size !== 3) {
      throw new Error(`STL preview triangle ${index / 3} collapses after vertex merging.`);
    }

    const [a, b, c] = [
      sourcePositions[mesh.indices[index]],
      sourcePositions[mesh.indices[index + 1]],
      sourcePositions[mesh.indices[index + 2]]
    ];
    const normal = new THREE.Vector3()
      .crossVectors(new THREE.Vector3().subVectors(b, a), new THREE.Vector3().subVectors(c, a));
    if (!Number.isFinite(normal.lengthSq()) || normal.length() <= minimumDoubleArea) {
      throw new Error(`STL preview triangle ${index / 3} is degenerate.`);
    }
    normal.normalize();
    faceNormals.push(normal);
    for (const vertexIndex of weldedIndices) weldedVertices[vertexIndex].faceNormals.push(normal);
  }

  const creaseDot = Math.cos(STL_CREASE_ANGLE_RADIANS);
  const positions: number[] = [];
  const normals: number[] = [];
  for (let index = 0; index < mesh.indices.length; index += 1) {
    const faceNormal = faceNormals[Math.floor(index / 3)];
    const vertex = weldedVertices[sourceToWelded[mesh.indices[index]]];
    const smoothNormal = new THREE.Vector3();
    for (const adjacentNormal of vertex.faceNormals) {
      if (faceNormal.dot(adjacentNormal) > creaseDot) smoothNormal.add(adjacentNormal);
    }
    if (smoothNormal.lengthSq() === 0) {
      throw new Error(`STL preview normal generation failed at triangle ${Math.floor(index / 3)}.`);
    }
    smoothNormal.normalize();
    const sourcePosition = sourcePositions[mesh.indices[index]];
    positions.push(sourcePosition.x, sourcePosition.y, sourcePosition.z);
    normals.push(smoothNormal.x, smoothNormal.y, smoothNormal.z);
  }

  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(positions, 3));
  geometry.setAttribute("normal", new THREE.Float32BufferAttribute(normals, 3));
  geometry.computeBoundingBox();
  geometry.computeBoundingSphere();
  const stats: StlPreviewGeometryStats = {
    sourceVertexCount,
    mergedVertexCount: weldedVertices.length,
    triangleCount: mesh.indices.length / 3,
    vertexMergeTolerance,
    creaseAngleRadians: STL_CREASE_ANGLE_RADIANS
  };
  geometry.userData.previewGeometry = stats;
  return geometry;
}

export function getStlPreviewGeometryStats(geometry: THREE.BufferGeometry): StlPreviewGeometryStats {
  const stats = geometry.userData.previewGeometry as StlPreviewGeometryStats | undefined;
  if (!stats) throw new Error("Geometry does not contain STL preview processing statistics.");
  return stats;
}

function validateMeshArrays(mesh: CadMesh): void {
  if (mesh.vertices.length === 0 || mesh.vertices.length % 3 !== 0) {
    throw new Error("STL preview vertices must contain complete XYZ coordinates.");
  }
  if (mesh.normals.length !== mesh.vertices.length) {
    throw new Error("STL preview normals must match the vertex coordinate count.");
  }
  if (mesh.indices.length === 0 || mesh.indices.length % 3 !== 0) {
    throw new Error("STL preview indices must contain complete triangles.");
  }
  for (let index = 0; index < mesh.vertices.length; index += 1) {
    if (!Number.isFinite(mesh.vertices[index])) {
      throw new Error(`STL preview vertex coordinate ${index} is not finite.`);
    }
  }
  for (let index = 0; index < mesh.normals.length; index += 1) {
    if (!Number.isFinite(mesh.normals[index])) {
      throw new Error(`STL preview source normal ${index} is not finite.`);
    }
  }
  const vertexCount = mesh.vertices.length / 3;
  for (let index = 0; index < mesh.indices.length; index += 1) {
    const vertexIndex = mesh.indices[index];
    if (!Number.isInteger(vertexIndex) || vertexIndex < 0 || vertexIndex >= vertexCount) {
      throw new Error(`STL preview index ${index} references an invalid vertex.`);
    }
  }
}

function weldVertices(
  positions: THREE.Vector3[],
  trianglePeers: Array<Set<number>>,
  tolerance: number
): { sourceToWelded: number[]; weldedVertices: WeldedVertex[] } {
  const cells = new Map<string, number[]>();
  const sourceToWelded: number[] = [];
  const weldedVertices: WeldedVertex[] = [];
  const toleranceSquared = tolerance * tolerance;

  for (let sourceIndex = 0; sourceIndex < positions.length; sourceIndex += 1) {
    const position = positions[sourceIndex];
    const cell = [
      Math.floor(position.x / tolerance),
      Math.floor(position.y / tolerance),
      Math.floor(position.z / tolerance)
    ];
    let weldedIndex: number | null = null;
    for (let x = -1; x <= 1 && weldedIndex === null; x += 1) {
      for (let y = -1; y <= 1 && weldedIndex === null; y += 1) {
        for (let z = -1; z <= 1 && weldedIndex === null; z += 1) {
          const candidates = cells.get(cellKey(cell[0] + x, cell[1] + y, cell[2] + z)) ?? [];
          for (const candidate of candidates) {
            const candidateVertex = weldedVertices[candidate];
            const sharesTriangle = candidateVertex.sourceIndices.some((candidateSourceIndex) => (
              trianglePeers[sourceIndex].has(candidateSourceIndex)
            ));
            if (
              !sharesTriangle
              && candidateVertex.position.distanceToSquared(position) <= toleranceSquared
            ) {
              weldedIndex = candidate;
              break;
            }
          }
        }
      }
    }
    if (weldedIndex === null) {
      weldedIndex = weldedVertices.length;
      weldedVertices.push({ position: position.clone(), faceNormals: [], sourceIndices: [sourceIndex] });
      const key = cellKey(cell[0], cell[1], cell[2]);
      const entries = cells.get(key) ?? [];
      entries.push(weldedIndex);
      cells.set(key, entries);
    } else {
      weldedVertices[weldedIndex].sourceIndices.push(sourceIndex);
    }
    sourceToWelded.push(weldedIndex);
  }
  return { sourceToWelded, weldedVertices };
}

function buildTrianglePeers(indices: number[], vertexCount: number): Array<Set<number>> {
  const trianglePeers = Array.from({ length: vertexCount }, () => new Set<number>());
  for (let index = 0; index < indices.length; index += 3) {
    const triangle = [indices[index], indices[index + 1], indices[index + 2]];
    for (const vertexIndex of triangle) {
      for (const peerIndex of triangle) {
        if (peerIndex !== vertexIndex) trianglePeers[vertexIndex].add(peerIndex);
      }
    }
  }
  return trianglePeers;
}

function cellKey(x: number, y: number, z: number): string {
  return `${x}:${y}:${z}`;
}
