import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const sidecarRoot = join(repoRoot, "src-tauri", "sidecars", "structural-anchor");
const rendererRoot = join(repoRoot, "src-tauri", "sidecars", "vlm-renderer");
const fixtureRoot = join(repoRoot, "fixtures", "structural-anchor");
const buildRoot = join(tmpdir(), "cadastrophe-structural-anchor-test-build");
const rendererBuildRoot = join(tmpdir(), "cadastrophe-vlm-renderer-test-build");
const executableName = process.platform === "win32"
  ? "cadastrophe-structural-anchor.exe"
  : "cadastrophe-structural-anchor";
const executablePath = join(buildRoot, executableName);
const rendererExecutableName = process.platform === "win32"
  ? "cadastrophe-vlm-renderer.exe"
  : "cadastrophe-vlm-renderer";
const rendererExecutablePath = join(rendererBuildRoot, rendererExecutableName);

test("structural anchor sidecar emits deterministic report for fixture STL", () => {
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

test("VLM renderer sidecar emits non-empty 9-view PNG manifest for fixture STL", () => {
  const executable = buildRendererSidecar();
  const outputDir = join(tmpdir(), `cadastrophe-vlm-renderer-${process.pid}`);
  mkdirSync(outputDir, { recursive: true });
  const input = {
    runId: "run-fixture",
    revisionId: "revision-fixture",
    artifactId: "artifact-final-stl",
    sourceArtifactSha256: "fixture-sha",
    sourceHash: "fixture-source",
    stlPath: join(fixtureRoot, "cube.stl"),
    outputDirectory: outputDir,
    viewMode: "9-view",
    resolution: { width: 128, height: 128 }
  };

  const result = spawnSync(executable, [], {
    cwd: repoRoot,
    encoding: "utf8",
    input: JSON.stringify(input)
  });

  assert.equal(result.status, 0, result.stderr);
  const manifest = JSON.parse(result.stdout);
  assert.equal(manifest.contractType, "cadastrophe.vlm_render_manifest.v1");
  assert.equal(manifest.format, "png");
  assert.equal(manifest.renderer, "cadastrophe-vlm-renderer");
  assert.equal(manifest.rendererEngine, "native-cpp-rasterizer");
  assert.equal(manifest.viewMode, "9-view");
  assert.deepEqual(manifest.views, [
    "Front-Left-Top",
    "Front",
    "Front-Right-Top",
    "Left",
    "Top",
    "Right",
    "Bottom",
    "Back",
    "Back-Right-Top"
  ]);
  assert.ok(manifest.bytes > 0);
  assert.ok(Number.isInteger(manifest.bytes));
  assert.equal(typeof manifest.sha256, "string");
  assert.ok(existsSync(manifest.path));
  const png = readFileSync(manifest.path);
  assert.deepEqual([...png.subarray(0, 8)], [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
});

function buildSidecar() {
  mkdirSync(buildRoot, { recursive: true });
  execFileSync("cmake", [
    "-S",
    sidecarRoot,
    "-B",
    buildRoot
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

function buildRendererSidecar() {
  mkdirSync(rendererBuildRoot, { recursive: true });
  execFileSync("cmake", [
    "-S",
    rendererRoot,
    "-B",
    rendererBuildRoot
  ], {
    cwd: repoRoot,
    stdio: "pipe"
  });
  execFileSync("cmake", ["--build", rendererBuildRoot], {
    cwd: repoRoot,
    stdio: "pipe"
  });
  return rendererExecutablePath;
}

function normalizeReport(report: any) {
  const manifestCheck = report.checks.find((check: { name: string }) => check.name === "artifact_manifest");
  if (manifestCheck) {
    manifestCheck.stlPath = "<fixture>/cube.stl";
  }
  return report;
}
