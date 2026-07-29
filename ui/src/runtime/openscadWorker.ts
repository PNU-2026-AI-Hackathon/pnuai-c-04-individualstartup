import { createOpenSCAD, type OpenSCAD } from "openscad-wasm";
import { parseStlToMesh } from "./stl";
import type { CadDiagnostic, CadDiagnostics, CadMesh, CadParameter } from "../protocol";

type RenderRequest = {
  type: "render";
  token: number;
  source: string;
  parameters: CadParameter[];
};

type CancelRequest = {
  type: "cancel";
  token: number;
};

type WorkerRequest = RenderRequest | CancelRequest;

export type OpenscadWorkerResponse =
  | { type: "initialized"; token: number }
  | {
      type: "rendered";
      token: number;
      diagnostics: CadDiagnostics;
      mesh: CadMesh;
      stlBytes: Uint8Array;
      sourceHash: string;
      parameterHash: string;
      stlSha256: string;
    }
  | { type: "failed"; token: number; diagnostics: CadDiagnostics };

let modulePromise: Promise<OpenSCAD> | undefined;
let activeToken = 0;

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  const message = event.data;
  activeToken = message.token;
  if (message.type === "cancel") return;
  render(message).catch((error) => {
    if (message.token !== activeToken) return;
    postMessage({
      type: "failed",
      token: message.token,
      diagnostics: diagnostics(false, 0, [
        {
          severity: "error",
          message: error instanceof Error ? error.message : String(error)
        }
      ])
    } satisfies OpenscadWorkerResponse);
  });
};

async function render(message: RenderRequest) {
  const startedAt = performance.now();
  const stdout: string[] = [];
  const stderr: string[] = [];
  const openscad = await getModule(stdout, stderr);
  if (message.token !== activeToken) return;
  postMessage({ type: "initialized", token: message.token } satisfies OpenscadWorkerResponse);

  const inputPath = `/cadastrophe-${message.token}.scad`;
  const outputPath = `/cadastrophe-${message.token}.stl`;
  try {
    openscad.FS.writeFile(inputPath, applyParameters(message.source, message.parameters));
    const exitCode = openscad.callMain([inputPath, "--enable=manifold", "-o", outputPath]);
    if (message.token !== activeToken) return;
    if (exitCode !== 0) {
      postMessage({
        type: "failed",
        token: message.token,
        diagnostics: diagnostics(false, elapsed(startedAt), [
          ...diagnosticsFromLines("info", stdout),
          ...diagnosticsFromLines("error", stderr),
          { severity: "error", message: `OpenSCAD exited with status ${exitCode}.` }
        ])
      } satisfies OpenscadWorkerResponse);
      return;
    }
    const stlBytes = openscad.FS.readFile(outputPath, { encoding: "binary" });
    const mesh = parseStlToMesh(stlBytes);
    const sourceHash = await sha256Hex(message.source);
    const parameterHash = await sha256Hex(JSON.stringify(parameterValues(message.parameters)));
    const stlSha256 = await sha256Hex(stlBytes);
    postMessage({
      type: "rendered",
      token: message.token,
      diagnostics: diagnostics(true, elapsed(startedAt), [
        ...diagnosticsFromLines("info", stdout),
        ...diagnosticsFromLines("warning", stderr)
      ]),
      mesh,
      stlBytes,
      sourceHash,
      parameterHash,
      stlSha256
    } satisfies OpenscadWorkerResponse);
  } finally {
    tryUnlink(openscad, inputPath);
    tryUnlink(openscad, outputPath);
  }
}

function getModule(stdout: string[], stderr: string[]) {
  if (!modulePromise) {
    modulePromise = createOpenSCAD({
      noInitialRun: true,
      print: (text) => stdout.push(text),
      printErr: (text) => stderr.push(text)
    }).then((instance) => instance.getInstance());
  }
  return modulePromise;
}

function applyParameters(source: string, parameters: CadParameter[]) {
  if (parameters.length === 0) return source;
  const values = new Map(parameters.map((parameter) => [parameter.name, scadLiteral(parameter.value)]));
  return source
    .split(/\r?\n/)
    .map((line) => {
      const match = /^(\s*([A-Za-z_]\w*)\s*=\s*)([^;]*)(;.*\/\/\s*@param\b.*)$/.exec(line);
      if (!match || !values.has(match[2])) return line;
      return `${match[1]}${values.get(match[2])}${match[4]}`;
    })
    .join("\n");
}

function scadLiteral(value: CadParameter["value"]) {
  if (typeof value === "string") return JSON.stringify(value);
  return String(value);
}

function parameterValues(parameters: CadParameter[]) {
  return Object.fromEntries(parameters.map((parameter) => [parameter.name, parameter.value]).sort());
}

function diagnostics(ok: boolean, elapsedMs: number, items: CadDiagnostic[]): CadDiagnostics {
  return { ok, elapsedMs, items };
}

function diagnosticsFromLines(severity: CadDiagnostic["severity"], lines: string[]): CadDiagnostic[] {
  return lines
    .map((line) => line.trim())
    .filter(Boolean)
    .map((message) => ({ severity, message }));
}

function elapsed(startedAt: number) {
  return Math.max(0, Math.round(performance.now() - startedAt));
}

async function sha256Hex(input: string | Uint8Array) {
  const bytes = typeof input === "string" ? new TextEncoder().encode(input) : new Uint8Array(input);
  const digest = await crypto.subtle.digest("SHA-256", bytes.buffer as ArrayBuffer);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

function tryUnlink(openscad: OpenSCAD, path: string) {
  try {
    openscad.FS.unlink(path);
  } catch {
    // OpenSCAD may not have created the output file on failed renders.
  }
}
