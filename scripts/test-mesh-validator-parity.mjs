#!/usr/bin/env node
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const open3dDir = resolve(
  process.env.OPEN3D_DIR ?? join(repoRoot, "Open3D", "install", "lib", "cmake", "Open3D")
);
const configPath = join(open3dDir, "Open3DConfig.cmake");
if (!existsSync(configPath)) {
  throw new Error(`Open3D reference build not found: ${configPath}`);
}

const buildRoot = join(tmpdir(), "cadastrophe-mesh-validator-parity");
mkdirSync(buildRoot, { recursive: true });
execFileSync("cmake", [
  "-S", join(repoRoot, "tests", "mesh-validator-parity"),
  "-B", buildRoot,
  `-DOpen3D_DIR=${open3dDir}`
], { cwd: repoRoot, stdio: "inherit" });
execFileSync("cmake", ["--build", buildRoot, "--parallel"], {
  cwd: repoRoot,
  stdio: "inherit"
});
execFileSync("ctest", ["--test-dir", buildRoot, "--output-on-failure"], {
  cwd: repoRoot,
  stdio: "inherit"
});
