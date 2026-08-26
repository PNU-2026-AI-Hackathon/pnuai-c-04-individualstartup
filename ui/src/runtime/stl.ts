import * as THREE from "three";
import type { CadMesh } from "../protocol";

export function parseStlToMesh(bytes: Uint8Array): CadMesh {
  if (isBinaryStl(bytes)) {
    return parseBinaryStl(bytes);
  }
  return parseAsciiStl(new TextDecoder().decode(bytes));
}

function isBinaryStl(bytes: Uint8Array) {
  if (bytes.byteLength < 84) return false;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const triangleCount = view.getUint32(80, true);
  return 84 + triangleCount * 50 === bytes.byteLength;
}

function parseBinaryStl(bytes: Uint8Array): CadMesh {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const triangleCount = view.getUint32(80, true);
  const vertices: number[] = [];
  const normals: number[] = [];
  const indices: number[] = [];
  let offset = 84;
  for (let triangle = 0; triangle < triangleCount; triangle += 1) {
    const normal = [
      view.getFloat32(offset, true),
      view.getFloat32(offset + 4, true),
      view.getFloat32(offset + 8, true)
    ];
    offset += 12;
    for (let vertex = 0; vertex < 3; vertex += 1) {
      vertices.push(
        view.getFloat32(offset, true),
        view.getFloat32(offset + 4, true),
        view.getFloat32(offset + 8, true)
      );
      normals.push(normal[0], normal[1], normal[2]);
      indices.push(indices.length);
      offset += 12;
    }
    offset += 2;
  }
  return normalizeMeshNormals({ vertices, normals, indices });
}

function parseAsciiStl(source: string): CadMesh {
  const vertices: number[] = [];
  const normals: number[] = [];
  const indices: number[] = [];
  const vertexPattern = /^\s*vertex\s+([-+.\deE]+)\s+([-+.\deE]+)\s+([-+.\deE]+)/i;
  for (const line of source.split(/\r?\n/)) {
    const match = vertexPattern.exec(line);
    if (!match) continue;
    vertices.push(Number(match[1]), Number(match[2]), Number(match[3]));
    indices.push(indices.length);
  }
  for (let index = 0; index < vertices.length; index += 9) {
    const a = new THREE.Vector3(vertices[index], vertices[index + 1], vertices[index + 2]);
    const b = new THREE.Vector3(vertices[index + 3], vertices[index + 4], vertices[index + 5]);
    const c = new THREE.Vector3(vertices[index + 6], vertices[index + 7], vertices[index + 8]);
    const normal = new THREE.Vector3().subVectors(b, a).cross(new THREE.Vector3().subVectors(c, a)).normalize();
    normals.push(normal.x, normal.y, normal.z, normal.x, normal.y, normal.z, normal.x, normal.y, normal.z);
  }
  return { vertices, normals, indices };
}

function normalizeMeshNormals(mesh: CadMesh): CadMesh {
  if (mesh.normals.some((value) => Number.isFinite(value) && Math.abs(value) > 0)) {
    return mesh;
  }
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(mesh.vertices, 3));
  geometry.setIndex(mesh.indices);
  geometry.computeVertexNormals();
  const normals = Array.from(geometry.getAttribute("normal").array);
  geometry.dispose();
  return { ...mesh, normals };
}
