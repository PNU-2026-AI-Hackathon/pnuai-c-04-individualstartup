import type { CadDiagnostics, CadMesh, CadParameter } from "../protocol";
import type { OpenscadWorkerResponse } from "./openscadWorker";

export type OpenscadRuntimeState = "idle" | "initializing" | "rendering" | "completed" | "failed" | "canceled";

export interface OpenscadRenderResult {
  diagnostics: CadDiagnostics;
  mesh: CadMesh;
  stlBytes: Uint8Array;
  sourceHash: string;
  parameterHash: string;
  stlSha256: string;
}

let worker: Worker | undefined;
let token = 0;

export function renderOpenScadInWorker(
  source: string,
  parameters: CadParameter[],
  onState: (state: OpenscadRuntimeState) => void
): Promise<OpenscadRenderResult> {
  const renderToken = token + 1;
  token = renderToken;
  const runtimeWorker = getWorker();
  onState("initializing");
  return new Promise((resolve, reject) => {
    const onMessage = (event: MessageEvent<OpenscadWorkerResponse>) => {
      const message = event.data;
      if (message.token !== renderToken) return;
      if (message.type === "initialized") {
        onState("rendering");
        return;
      }
      runtimeWorker.removeEventListener("message", onMessage);
      if (message.type === "rendered") {
        onState("completed");
        resolve(message);
        return;
      }
      onState("failed");
      reject(new OpenscadRuntimeError(message.diagnostics));
    };
    runtimeWorker.addEventListener("message", onMessage);
    runtimeWorker.postMessage({ type: "render", token: renderToken, source, parameters });
  });
}

export function cancelOpenScadRender() {
  token += 1;
  worker?.postMessage({ type: "cancel", token });
}

class OpenscadRuntimeError extends Error {
  constructor(readonly diagnostics: CadDiagnostics) {
    super(diagnostics.items.map((item) => item.message).find(Boolean) ?? "OpenSCAD render failed.");
  }
}

function getWorker() {
  worker ??= new Worker(new URL("./openscadWorker.ts", import.meta.url), { type: "module" });
  return worker;
}
