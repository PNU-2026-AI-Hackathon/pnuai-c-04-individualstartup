import { readFile } from "node:fs/promises";
import { createOpenSCAD } from "openscad-wasm";

const sourcePath = process.argv[2];
if (!sourcePath) {
  console.error("usage: node scripts/openscad-render.mjs <source.scad>");
  process.exit(2);
}

const startedAt = performance.now();
const stdout = [];
const stderr = [];

try {
  const source = await readFile(sourcePath, "utf8");
  const api = await createOpenSCAD({
    noInitialRun: true,
    print: (text) => stdout.push(String(text)),
    printErr: (text) => stderr.push(String(text))
  });
  const openscad = api.getInstance();
  const inputPath = "/cadgen-ax-input.scad";
  const outputPath = "/cadgen-ax-output.stl";
  openscad.FS.writeFile(inputPath, source);
  const exitCode = openscad.callMain([inputPath, "--backend=manifold", "-o", outputPath]);
  if (exitCode !== 0) {
    writeResult({
      diagnostics: diagnostics(false, [
        ...diagnosticsFromLines("info", stdout),
        ...diagnosticsFromLines("error", stderr),
        { severity: "error", message: `OpenSCAD exited with status ${exitCode}.` }
      ])
    });
    process.exit(0);
  }
  const stlBytes = openscad.FS.readFile(outputPath, { encoding: "binary" });
  writeResult({
    diagnostics: diagnostics(true, [
      ...diagnosticsFromLines("info", stdout),
      ...diagnosticsFromLines("warning", stderr)
    ]),
    mesh: parseStlToMesh(stlBytes),
    stlBase64: Buffer.from(stlBytes).toString("base64"),
    stlSha256: await sha256Hex(stlBytes),
    stlBytes: stlBytes.byteLength
  });
} catch (error) {
  writeResult({
    diagnostics: diagnostics(false, [
      ...diagnosticsFromLines("info", stdout),
      ...diagnosticsFromLines("error", stderr),
      { severity: "error", message: error instanceof Error ? error.message : String(error) }
    ])
  });
}

function writeResult(result) {
  process.stdout.write(JSON.stringify(result));
}

function diagnostics(ok, items) {
  return {
    ok,
    elapsedMs: Math.max(0, Math.round(performance.now() - startedAt)),
    items
  };
}

function diagnosticsFromLines(severity, lines) {
  return lines
    .map((line) => line.trim())
    .filter(Boolean)
    .map((message) => ({ severity, message }));
}

async function sha256Hex(bytes) {
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function parseStlToMesh(bytes) {
  if (isBinaryStl(bytes)) return parseBinaryStl(bytes);
  return parseAsciiStl(new TextDecoder().decode(bytes));
}

function isBinaryStl(bytes) {
  if (bytes.byteLength < 84) return false;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const triangleCount = view.getUint32(80, true);
  return 84 + triangleCount * 50 === bytes.byteLength;
}

function parseBinaryStl(bytes) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const triangleCount = view.getUint32(80, true);
  const vertices = [];
  const normals = [];
  const indices = [];
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
  return { vertices, normals, indices };
}

function parseAsciiStl(source) {
  const vertices = [];
  const normals = [];
  const indices = [];
  const vertexPattern = /^\s*vertex\s+([-+.\deE]+)\s+([-+.\deE]+)\s+([-+.\deE]+)/i;
  for (const line of source.split(/\r?\n/)) {
    const match = vertexPattern.exec(line);
    if (!match) continue;
    vertices.push(Number(match[1]), Number(match[2]), Number(match[3]));
    indices.push(indices.length);
  }
  for (let index = 0; index < vertices.length; index += 9) {
    const normal = triangleNormal(
      [vertices[index], vertices[index + 1], vertices[index + 2]],
      [vertices[index + 3], vertices[index + 4], vertices[index + 5]],
      [vertices[index + 6], vertices[index + 7], vertices[index + 8]]
    );
    normals.push(...normal, ...normal, ...normal);
  }
  return { vertices, normals, indices };
}

function triangleNormal(a, b, c) {
  const ux = b[0] - a[0];
  const uy = b[1] - a[1];
  const uz = b[2] - a[2];
  const vx = c[0] - a[0];
  const vy = c[1] - a[1];
  const vz = c[2] - a[2];
  const nx = uy * vz - uz * vy;
  const ny = uz * vx - ux * vz;
  const nz = ux * vy - uy * vx;
  const length = Math.hypot(nx, ny, nz) || 1;
  return [nx / length, ny / length, nz / length];
}
