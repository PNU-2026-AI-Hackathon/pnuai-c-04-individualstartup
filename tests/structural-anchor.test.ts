import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sidecarRoot = join(repoRoot, "src-tauri", "sidecars", "structural-anchor");
const fixtureRoot = join(repoRoot, "fixtures", "structural-anchor");
const buildRoot = join(tmpdir(), "cadastrophe-structural-anchor-test-build");
const executableName = process.platform === "win32"
  ? "cadastrophe-structural-anchor.exe"
  : "cadastrophe-structural-anchor";
const executablePath = join(buildRoot, executableName);

test("structural anchor sidecar emits deterministic fallback report for fixture STL", () => {
  const executable = buildSidecar();
  const output = execFileSync(executable, [
    "--input",
    join(fixtureRoot, "cube_input.v1.json")
  ], {
    cwd: repoRoot,
    encoding: "utf8"
  });

  const actual = normalizeReport(JSON.parse(output));
  const expected = JSON.parse(readFileSync(join(fixtureRoot, "cube_report.v1.json"), "utf8"));
  assert.deepEqual(actual, expected);
});

test("structural anchor sidecar accepts stdin input contract", () => {
  const executable = buildSidecar();
  const input = JSON.parse(readFileSync(join(fixtureRoot, "cube_input.v1.json"), "utf8"));
  input.planPath = join(fixtureRoot, "cube_plan.v1.json");
  input.stlPath = join(fixtureRoot, "cube.stl");

  const result = spawnSync(executable, [], {
    cwd: repoRoot,
    encoding: "utf8",
    input: JSON.stringify(input)
  });

  assert.equal(result.status, 0, result.stderr);
  const report = normalizeReport(JSON.parse(result.stdout));
  assert.equal(report.contractType, "cadastrophe.structural_report.v1");
  assert.equal(report.passed, true);
  assert.deepEqual(
    report.checks.map((check: { name: string }) => check.name),
    [
      "main_component",
      "runtime_diagnostics",
      "artifact_manifest",
      "stl_load",
      "mesh_non_empty",
      "mesh_cleanup",
      "topology",
      "aspect_ratio"
    ]
  );
});

function buildSidecar() {
  mkdirSync(buildRoot, { recursive: true });
  execFileSync("cmake", [
    "-S",
    sidecarRoot,
    "-B",
    buildRoot,
    "-DCADASTROPHE_STRUCTURAL_ANCHOR_USE_OPEN3D=OFF"
  ], {
    cwd: repoRoot,
    stdio: "pipe"
  });
  execFileSync("cmake", ["--build", buildRoot], {
    cwd: repoRoot,
    stdio: "pipe"
  });
  return executablePath;
}

function normalizeReport(report: any) {
  const manifestCheck = report.checks.find((check: { name: string }) => check.name === "artifact_manifest");
  if (manifestCheck) {
    manifestCheck.stlPath = "<fixture>/cube.stl";
  }
  return report;
}
