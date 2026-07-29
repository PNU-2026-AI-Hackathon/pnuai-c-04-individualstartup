#!/usr/bin/env node
import { copyFileSync, existsSync, mkdirSync, chmodSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = parseArgs(process.argv.slice(2));
const profile = args.profile ?? "debug";
const targetRoot = resolve(repoRoot, args.targetDir ?? join("src-tauri", "target"));
const sidecarRoot = join(repoRoot, "src-tauri", "sidecars", "structural-anchor");
const buildRoot = resolve(
  repoRoot,
  args.buildDir ?? join("src-tauri", "target", "structural-anchor", profile)
);
const executableName = process.platform === "win32"
  ? "cadastrophe-structural-anchor.exe"
  : "cadastrophe-structural-anchor";
const builtExecutable = join(buildRoot, executableName);
const adjacentExecutable = join(targetRoot, profile, executableName);

main();

function main() {
  requireCommand("cmake");
  mkdirSync(buildRoot, { recursive: true });
  mkdirSync(dirname(adjacentExecutable), { recursive: true });

  run("cmake", [
    "-S",
    sidecarRoot,
    "-B",
    buildRoot,
    "-DCADASTROPHE_STRUCTURAL_ANCHOR_USE_OPEN3D=OFF",
    `-DCMAKE_BUILD_TYPE=${profile === "release" ? "Release" : "Debug"}`
  ]);
  run("cmake", ["--build", buildRoot, "--config", profile === "release" ? "Release" : "Debug"]);

  if (!existsSync(builtExecutable)) {
    throw new Error(`Structural sidecar build did not produce ${builtExecutable}`);
  }
  copyFileSync(builtExecutable, adjacentExecutable);
  if (process.platform !== "win32") {
    chmodSync(adjacentExecutable, 0o755);
  }
  console.log(`[build-structural-sidecar] installed ${adjacentExecutable}`);
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

function requireCommand(command) {
  const result = spawnSync(command, ["--version"], { encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw new Error(`Required command not available: ${command}`);
  }
}

function run(command, commandArgs) {
  const result = spawnSync(command, commandArgs, {
    cwd: repoRoot,
    stdio: "inherit"
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${commandArgs.join(" ")} failed with status ${result.status}`);
  }
}
