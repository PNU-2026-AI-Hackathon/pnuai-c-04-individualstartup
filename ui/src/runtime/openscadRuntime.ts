import type { CadDiagnostics, CadMesh, CadParameter } from "../protocol";
import type { OpenscadWorkerResponse } from "./openscadWorker";

export type OpenscadRuntimeState = "idle" | "initializing" | "rendering" | "completed" | "failed" | "canceled";
export type RenderFailureOrigin =
  | "openscad-stderr"
  | "worker-throw"
  | "stl-parse"
  | "tauri-persistence"
  | "stale-render";

export interface OpenscadRenderResult {
  sessionId: string;
  revisionId: string;
  diagnostics: CadDiagnostics;
  mesh: CadMesh;
  stlBytes: Uint8Array;
  sourceHash: string;
  parameterHash: string;
  stlSha256: string;
}

export interface OpenscadRenderRequest {
  sessionId: string;
  revisionId: string;
  source: string;
  parameters: CadParameter[];
  sourceHash: string;
  parameterHash: string;
}

export type OpenscadPreviewCacheKey = Pick<
  OpenscadRenderRequest,
  "sessionId" | "revisionId" | "sourceHash" | "parameterHash"
>;

export function matchesOpenscadPreviewCache(
  cache: OpenscadRenderResult | null | undefined,
  key: OpenscadPreviewCacheKey
): cache is OpenscadRenderResult {
  return Boolean(
    cache &&
      cache.sessionId === key.sessionId &&
      cache.revisionId === key.revisionId &&
      cache.sourceHash === key.sourceHash &&
      cache.parameterHash === key.parameterHash
  );
}

let worker: Worker | undefined;
let workerSessionId: string | undefined;
let token = 0;
let activeRender:
  | {
      token: number;
      sessionId: string;
      runtimeWorker: Worker;
      onMessage: (event: MessageEvent<OpenscadWorkerResponse>) => void;
      reject: (reason?: unknown) => void;
      onState: (state: OpenscadRuntimeState) => void;
    }
  | undefined;

export function renderOpenScadInWorker(
  request: OpenscadRenderRequest,
  onState: (state: OpenscadRuntimeState) => void
): Promise<OpenscadRenderResult> {
  cancelActiveRender(false);
  const renderToken = token + 1;
  token = renderToken;
  const runtimeWorker = getWorker(request.sessionId);
  onState("initializing");
  return new Promise((resolve, reject) => {
    const onMessage = (event: MessageEvent<OpenscadWorkerResponse>) => {
      const message = event.data;
      if (message.token !== renderToken) return;
      if (message.sessionId !== request.sessionId || message.revisionId !== request.revisionId) return;
      if (message.type === "initialized") {
        onState("rendering");
        return;
      }
      clearActiveRender(renderToken);
      if (message.type === "rendered") {
        onState("completed");
        resolve(message);
        return;
      }
      onState("failed");
      logRenderFailureDiagnostics(message.diagnostics);
      reject(new OpenscadRuntimeError(message.diagnostics));
    };
    runtimeWorker.addEventListener("message", onMessage);
    activeRender = { token: renderToken, sessionId: request.sessionId, runtimeWorker, onMessage, reject, onState };
    runtimeWorker.postMessage({ type: "render", token: renderToken, ...request });
  });
}

export function cancelOpenScadRender() {
  token += 1;
  cancelActiveRender(true);
  worker?.postMessage({ type: "cancel", token, sessionId: workerSessionId });
}

export function resetOpenScadRuntimeForSessionSwitch() {
  token += 1;
  cancelActiveRender(true);
  worker?.terminate();
  worker = undefined;
  workerSessionId = undefined;
}

export class OpenscadRuntimeError extends Error {
  constructor(readonly diagnostics: CadDiagnostics) {
    super(diagnostics.items.map((item) => item.message).find(Boolean) ?? "OpenSCAD render failed.");
  }
}

export class OpenscadRenderCanceledError extends Error {
  constructor() {
    super("OpenSCAD render canceled.");
  }
}

export function diagnosticsFromOpenScadError(error: unknown): CadDiagnostics | undefined {
  return error instanceof OpenscadRuntimeError ? error.diagnostics : undefined;
}

export function createRenderFailureDiagnostics(input: {
  origin: RenderFailureOrigin;
  code?: string | number;
  message: string;
  sessionId?: string;
  revisionId?: string;
  sourceHash?: string;
  parameterHash?: string;
  elapsedMs?: number;
}): CadDiagnostics {
  return {
    ok: false,
    elapsedMs: input.elapsedMs ?? 0,
    items: [
      {
        severity: "error",
        message: renderFailureMessage(input)
      }
    ]
  };
}

export function logRenderFailureDiagnostics(diagnostics: CadDiagnostics): void {
  if (diagnostics.ok) return;
  const firstMessage = diagnostics.items.find((item) => item.severity === "error")?.message;
  console.error("Cadastrophe render failure", {
    ok: diagnostics.ok,
    elapsedMs: diagnostics.elapsedMs,
    message: firstMessage,
    items: diagnostics.items
  });
}

export function isOpenScadRenderCanceled(error: unknown): boolean {
  return error instanceof OpenscadRenderCanceledError;
}

export function nextRenderGeneration(current: number): number {
  return current + 1;
}

export function isLatestRenderGeneration(current: number, generation?: number): boolean {
  return generation === undefined || current === generation;
}

function cancelActiveRender(updateState: boolean) {
  const pending = activeRender;
  if (!pending) return;
  pending.runtimeWorker.removeEventListener("message", pending.onMessage);
  activeRender = undefined;
  if (updateState) pending.onState("canceled");
  pending.reject(new OpenscadRenderCanceledError());
}

function clearActiveRender(renderToken: number) {
  if (activeRender?.token !== renderToken) return;
  activeRender.runtimeWorker.removeEventListener("message", activeRender.onMessage);
  activeRender = undefined;
}

function getWorker(sessionId: string) {
  if (worker && workerSessionId !== sessionId) {
    worker.terminate();
    worker = undefined;
  }
  if (!worker) {
    worker = new Worker(new URL("./openscadWorker.ts", import.meta.url), { type: "module" });
    workerSessionId = sessionId;
  }
  return worker;
}

function renderFailureMessage(input: {
  origin: RenderFailureOrigin;
  code?: string | number;
  message: string;
  sessionId?: string;
  revisionId?: string;
  sourceHash?: string;
  parameterHash?: string;
}): string {
  const parts = [
    `origin=${input.origin}`,
    `code=${input.code ?? "unknown"}`,
    `message=${JSON.stringify(input.message)}`,
    `session=${input.sessionId ?? "unknown"}`,
    `revision=${input.revisionId ?? "unknown"}`,
    `sourceHash=${input.sourceHash ?? "unknown"}`,
    `parameterHash=${input.parameterHash ?? "unknown"}`
  ];
  return `Render failure diagnostics: ${parts.join(" ")}`;
}
