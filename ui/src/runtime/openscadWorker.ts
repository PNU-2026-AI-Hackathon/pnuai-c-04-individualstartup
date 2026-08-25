import { createOpenSCAD, type OpenSCAD } from "openscad-wasm";
import { parseStlToMesh } from "./stl";
import type { CadDiagnostic, CadDiagnostics, CadMesh, CadParameter } from "../protocol";
import { parameterHashInput } from "./parameterMetadata";

type RenderRequest = {
  type: "render";
  token: number;
  sessionId: string;
  revisionId: string;
  source: string;
  parameters: CadParameter[];
  sourceHash: string;
  parameterHash: string;
};

type CancelRequest = {
  type: "cancel";
  token: number;
  sessionId?: string;
};

type WorkerRequest = RenderRequest | CancelRequest;

export type OpenscadWorkerResponse =
  | { type: "initialized"; token: number; sessionId: string; revisionId: string }
  | {
      type: "rendered";
      token: number;
      sessionId: string;
      revisionId: string;
      diagnostics: CadDiagnostics;
      mesh: CadMesh;
      stlBytes: Uint8Array;
      sourceHash: string;
      parameterHash: string;
      stlSha256: string;
    }
  | { type: "failed"; token: number; sessionId: string; revisionId: string; diagnostics: CadDiagnostics };

let modulePromise: Promise<OpenSCAD> | undefined;
let activeToken = 0;
const moduleStdout: string[] = [];
const moduleStderr: string[] = [];

self.onmessage = (event: MessageEvent<WorkerRequest>) => {
  const message = event.data;
  activeToken = message.token;
  if (message.type === "cancel") return;
  render(message).catch((error) => {
    if (message.token !== activeToken) return;
    postMessage({
      type: "failed",
      token: message.token,
      sessionId: message.sessionId,
      revisionId: message.revisionId,
      diagnostics: failureDiagnostics({
        origin: "worker-throw",
        message,
        code: errorCode(error),
        detail: errorMessage(error),
        elapsedMs: 0
      })
    } satisfies OpenscadWorkerResponse);
  });
};

async function render(message: RenderRequest) {
  const startedAt = performance.now();
  clearModuleOutput();
  let appliedSource: string;
  let sourceHash: string;
  let parameterHash: string;
  try {
    appliedSource = message.source;
    sourceHash = await sha256Hex(appliedSource);
    parameterHash = await sha256Hex(parameterHashInput(message.parameters));
  } catch (error) {
    postFailure(message, failureDiagnostics({
      origin: "worker-throw",
      message,
      code: errorCode(error),
      detail: errorMessage(error),
      elapsedMs: elapsed(startedAt)
    }));
    return;
  }
  if (sourceHash !== message.sourceHash || parameterHash !== message.parameterHash) {
    postFailure(message, failureDiagnostics({
      origin: "stale-render",
      message,
      code: "render-identity-mismatch",
      detail: "OpenSCAD render identity changed before worker execution.",
      elapsedMs: elapsed(startedAt),
      actualSourceHash: sourceHash,
      actualParameterHash: parameterHash
    }));
    return;
  }

  let openscad: OpenSCAD;
  try {
    openscad = await getModule();
  } catch (error) {
    postFailure(message, failureDiagnostics({
      origin: "worker-throw",
      message,
      code: errorCode(error),
      detail: errorMessage(error),
      elapsedMs: elapsed(startedAt)
    }));
    return;
  }
  if (message.token !== activeToken) return;
  postMessage({
    type: "initialized",
    token: message.token,
    sessionId: message.sessionId,
    revisionId: message.revisionId
  } satisfies OpenscadWorkerResponse);

  const namespace = requestNamespace(message);
  const inputPath = `/cadgen-ax-${namespace}.scad`;
  const outputPath = `/cadgen-ax-${namespace}.stl`;
  try {
    openscad.FS.writeFile(inputPath, appliedSource);
    const exitCode = openscad.callMain([inputPath, "--backend=manifold", "-o", outputPath]);
    if (message.token !== activeToken) return;
    const stdout = drainModuleOutput(moduleStdout);
    const stderr = drainModuleOutput(moduleStderr);
    if (exitCode !== 0) {
      postMessage({
        type: "failed",
        token: message.token,
        sessionId: message.sessionId,
        revisionId: message.revisionId,
        diagnostics: failureDiagnostics({
          origin: "openscad-stderr",
          message,
          code: `openscad-exit-${exitCode}`,
          detail: stderr.find((line) => line.trim()) ?? `OpenSCAD exited with status ${exitCode}.`,
          elapsedMs: elapsed(startedAt),
          items: [
            ...diagnosticsFromLines("info", stdout),
            ...diagnosticsFromLines("error", stderr)
          ]
        })
      } satisfies OpenscadWorkerResponse);
      return;
    }
    let stlBytes: Uint8Array;
    let mesh: CadMesh;
    try {
      stlBytes = openscad.FS.readFile(outputPath, { encoding: "binary" });
      mesh = parseStlToMesh(stlBytes);
    } catch (error) {
      postFailure(message, failureDiagnostics({
        origin: "stl-parse",
        message,
        code: errorCode(error),
        detail: errorMessage(error),
        elapsedMs: elapsed(startedAt)
      }));
      return;
    }
    const stlSha256 = await sha256Hex(stlBytes);
    postMessage({
      type: "rendered",
      token: message.token,
      sessionId: message.sessionId,
      revisionId: message.revisionId,
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

function getModule() {
  if (!modulePromise) {
    modulePromise = createOpenSCAD({
      noInitialRun: true,
      print: (text) => moduleStdout.push(text),
      printErr: (text) => moduleStderr.push(text)
    }).then((instance) => instance.getInstance());
  }
  return modulePromise;
}

function clearModuleOutput() {
  moduleStdout.length = 0;
  moduleStderr.length = 0;
}

function drainModuleOutput(lines: string[]) {
  const output = [...lines];
  lines.length = 0;
  return output;
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

function requestNamespace(message: RenderRequest) {
  return [
    message.sessionId,
    message.revisionId,
    message.sourceHash.slice(0, 16),
    message.parameterHash.slice(0, 16),
    String(message.token)
  ].map((part) => part.replace(/[^A-Za-z0-9_.-]/g, "_")).join("-");
}

type FailureOrigin = "openscad-stderr" | "worker-throw" | "stl-parse" | "stale-render";

function postFailure(message: RenderRequest, diagnostics: CadDiagnostics): void {
  if (message.token !== activeToken) return;
  postMessage({
    type: "failed",
    token: message.token,
    sessionId: message.sessionId,
    revisionId: message.revisionId,
    diagnostics
  } satisfies OpenscadWorkerResponse);
}

function failureDiagnostics(input: {
  origin: FailureOrigin;
  message: RenderRequest;
  code?: string | number;
  detail: string;
  elapsedMs: number;
  items?: CadDiagnostic[];
  actualSourceHash?: string;
  actualParameterHash?: string;
}): CadDiagnostics {
  return diagnostics(false, input.elapsedMs, [
    ...(input.items ?? []),
    {
      severity: "error",
      message: renderFailureMessage(input)
    }
  ]);
}

function renderFailureMessage(input: {
  origin: FailureOrigin;
  message: RenderRequest;
  code?: string | number;
  detail: string;
  actualSourceHash?: string;
  actualParameterHash?: string;
}): string {
  const parts = [
    `origin=${input.origin}`,
    `code=${input.code ?? "unknown"}`,
    `message=${JSON.stringify(input.detail)}`,
    `session=${input.message.sessionId}`,
    `revision=${input.message.revisionId}`,
    `sourceHash=${input.message.sourceHash}`,
    `parameterHash=${input.message.parameterHash}`
  ];
  if (input.actualSourceHash) parts.push(`actualSourceHash=${input.actualSourceHash}`);
  if (input.actualParameterHash) parts.push(`actualParameterHash=${input.actualParameterHash}`);
  return `Render failure diagnostics: ${parts.join(" ")}`;
}

function errorCode(error: unknown): string | number | undefined {
  if (typeof error === "object" && error && "code" in error) {
    const code = (error as { code?: unknown }).code;
    if (typeof code === "string" || typeof code === "number") return code;
  }
  return extractNumericErrorCode(errorMessage(error));
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function extractNumericErrorCode(message: string): string | undefined {
  return /(?:error|code)[:\s]+(\d+)/i.exec(message)?.[1];
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
