#!/usr/bin/env node
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { buildSidecars, inferTargetTriple } from "./lib/sidecar-build.mjs";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = parseArgs(process.argv.slice(2));
const profile = args.profile ?? "debug";
const targetRoot = resolve(repoRoot, args.targetDir ?? join("src-tauri", "target"));
const targetTriple = args.targetTriple ?? inferTargetTriple();
const sidecars = [
  {
    name: "structural-anchor",
    executable: "cadastrophe-structural-anchor"
  },
  {
    name: "vlm-renderer",
    executable: "cadastrophe-vlm-renderer"
  }
];

main();

function main() {
  buildSidecars({
    repoRoot,
    profile,
    targetRoot,
    buildDir: args.buildDir,
    targetTriple,
    sidecars
  });
}

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--profile") {
      parsed.profile = requireValue(argv, ++index, arg);
    } else if (arg === "--target-dir") {
      parsed.targetDir = requireValue(argv, ++index, arg);
    } else if (arg === "--build-dir") {
      parsed.buildDir = requireValue(argv, ++index, arg);
    } else if (arg === "--target-triple") {
      parsed.targetTriple = requireValue(argv, ++index, arg);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }
  if (parsed.profile && !["debug", "release"].includes(parsed.profile)) {
    throw new Error("--profile must be debug or release");
  }
  return parsed;
}

function requireValue(argv, index, flag) {
  const value = argv[index];
  if (!value || value.startsWith("--")) {
    throw new Error(`${flag} requires a value`);
  }
  return value;
}
