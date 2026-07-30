import type { CadParameter } from "./protocol";
import { toHistoryPath } from "./navigation";
import type { OpenscadRenderResult } from "./runtime/openscadRuntime";

export function replaceUrl(uiUrl: string): void {
  window.history.replaceState({}, "", toHistoryPath(uiUrl, window.location.href));
}

export function runtimeMetadata(
  rendered: OpenscadRenderResult,
  phase: "preview" | "export"
): Record<string, unknown> {
  return {
    runtime: "openscad-wasm",
    sourceLanguage: "openscad",
    sourceHash: rendered.sourceHash,
    parameterHash: rendered.parameterHash,
    stlSha256: rendered.stlSha256,
    stlBytes: rendered.stlBytes.byteLength,
    renderDurationMs: rendered.diagnostics.elapsedMs,
    diagnosticsSource: "openscad-wasm",
    phase
  };
}

export function parameterValues(parameters: CadParameter[]) {
  return Object.fromEntries(parameters.map((parameter) => [parameter.name, parameter.value]).sort());
}

export async function sha256Hex(input: string | Uint8Array) {
  const bytes = typeof input === "string" ? new TextEncoder().encode(input) : new Uint8Array(input);
  const digest = await crypto.subtle.digest("SHA-256", bytes.buffer as ArrayBuffer);
  return Array.from(new Uint8Array(digest), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function base64EncodeUtf8(value: string) {
  return base64EncodeBytes(new TextEncoder().encode(value));
}

export function base64EncodeBytes(bytes: Uint8Array) {
  let binary = "";
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return btoa(binary);
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
