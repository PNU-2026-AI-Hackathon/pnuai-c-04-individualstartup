#!/usr/bin/env node
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const targetRoot = join(repoRoot, "src-tauri", "target", "debug");
const appDataDir = process.env.CADASTROPHE_CLI_SMOKE_APP_DATA_DIR
  ? resolve(process.env.CADASTROPHE_CLI_SMOKE_APP_DATA_DIR)
  : mkdtempSync(join(tmpdir(), "cadastrophe-cli-smoke-"));
const keepAppData = process.env.CADASTROPHE_CLI_SMOKE_KEEP_APP_DATA === "1";
const sessionId = "manual-cli-session-1";
const runId = "manual-cli-run-1";
const seedRevisionId = "manual-cli-seed-revision-1";
const sidecarName = process.platform === "win32"
  ? "cadastrophe-structural-anchor.exe"
  : "cadastrophe-structural-anchor";
const adjacentSidecarPath = join(targetRoot, sidecarName);

const plan = {
  schemaVersion: "cad_model_plan.v1",
  summary: "Manual CLI smoke bracket represented as one rectangular printable body.",
  mainComponent: {
    name: "wall_bracket",
    purpose: "single printable bracket body",
    requiredFeatures: ["rectangular_body"]
  },
  supportingComponents: [],
  expectedAspectRatio: { x: 3, y: 1, z: 2, tolerance: 0.35 },
  sourceLanguage: "openscad",
  runtimeConstraints: {
    runtime: "openscad-wasm",
    requiredFeatures: ["cube"],
    forbiddenFeatures: ["external_file_include"],
    mainComponentAnnotation: "// @main_component wall_bracket"
  }
};

const source = `// @main_component wall_bracket
width = 3; // @param min=1 max=10 step=1 label=Width
depth = 1; // @param min=1 max=10 step=1 label=Depth
height = 2; // @param min=1 max=10 step=1 label=Height

cube([width, depth, height]);
`;

main();

function main() {
  try {
    log(`repo: ${repoRoot}`);
    log(`app data: ${appDataDir}`);
    requireCommand("cargo");
    requireCommand("cmake");
    requireCommand("sqlite3");

    buildCliBins();
    buildStructuralSidecar();
    seedFixtureDatabase();

    const planPath = join(appDataDir, "plan.json");
    const sourcePath = join(appDataDir, "source.scad");
    writeJson(planPath, plan);
    writeFileSync(sourcePath, source);

    const current = runCli("cadastrophe-session-current");
    assertEqual(current.data.sessionId, sessionId, "session-current returns seeded session");

    const initialState = runCli("cadastrophe-session-state", "--session", sessionId);
    assertEqual(initialState.data.state.agentRuns[0].id, runId, "session-state returns seeded run");

    const committed = runCli(
      "cadastrophe-plan-commit",
      "--session", sessionId,
      "--run", runId,
      "--plan", planPath
    );
    assertEqual(committed.data.nextAction, "source_apply", "plan-commit nextAction");

    const applied = runCli(
      "cadastrophe-source-apply",
      "--session", sessionId,
      "--run", runId,
      "--source", sourcePath,
      "--language", "openscad"
    );
    const revisionId = requireString(applied.data.revisionId, "source-apply revisionId");

    const preview = runCli(
      "cadastrophe-preview-render",
      "--session", sessionId,
      "--run", runId,
      "--revision", revisionId
    );
    assertEqual(preview.data.diagnostics.ok, true, "preview-render diagnostics");

    const exported = runCli(
      "cadastrophe-artifact-export",
      "--session", sessionId,
      "--run", runId,
      "--revision", revisionId,
      "--format", "stl"
    );
    assertEqual(exported.data.diagnostics.ok, true, "artifact-export diagnostics");
    const exportedArtifactId = requireString(exported.data.artifact?.id, "artifact-export artifact id");

    const structural = runCli(
      "cadastrophe-evaluate-structural",
      "--session", sessionId,
      "--run", runId,
      "--revision", revisionId,
      "--plan", planPath,
      "--artifact", exportedArtifactId
    );
    assertEqual(structural.data.structuralReport.passed, true, "evaluate-structural passed");

    const finalized = runCli(
      "cadastrophe-finalize",
      "--session", sessionId,
      "--run", runId,
      "--revision", revisionId
    );
    assertEqual(finalized.data.next_action, "vlm_judge", "finalize next_action");
    assertEqual(finalized.data.vlmContract?.contractType, "cadastrophe.vlm_judge.v1", "finalize VLM contract");
    const finalArtifactId = requireString(finalized.data.artifactId, "finalize artifact id");

    const vlmReportPath = join(appDataDir, "vlm-report.json");
    writeJson(vlmReportPath, {
      contractType: "cadastrophe.vlm_judge_report.v1",
      runId,
      artifactId: finalArtifactId,
      score: 0.97,
      passed: true,
      findings: []
    });
    const submitted = runCli(
      "cadastrophe-vlm-submit",
      "--session", sessionId,
      "--run", runId,
      "--artifact", finalArtifactId,
      "--report", vlmReportPath
    );
    assertEqual(submitted.data.next_action, "complete", "vlm-submit next_action");

    const finalState = runCli("cadastrophe-session-state", "--session", sessionId);
    const completedCommands = finalState.data.state.agentRunEvents
      .filter((event) => event.runId === runId && event.type === "agent.tool.completed")
      .map((event) => event.payload.command)
      .filter(Boolean);
    for (const command of [
      "cadastrophe-plan-commit",
      "cadastrophe-source-apply",
      "cadastrophe-preview-render",
      "cadastrophe-artifact-export",
      "cadastrophe-evaluate-structural",
      "cadastrophe-finalize",
      "cadastrophe-vlm-submit"
    ]) {
      assert(completedCommands.includes(command), `run events include ${command}`);
    }
    assertEqual(finalState.data.state.workflow.outerIterations.at(-1)?.passed, true, "outer iteration pass persisted");
    assertEqual(finalState.data.state.workflow.pendingVlm.length, 0, "pending VLM consumed");

    log("all CLI manual smoke checks passed");
    if (keepAppData || process.env.CADASTROPHE_CLI_SMOKE_APP_DATA_DIR) {
      log(`inspect fixture state with: ${binPath("cadastrophe-session-state")} --app-data-dir ${appDataDir} --session ${sessionId} --pretty`);
    } else {
      log("temporary app data cleaned up; set CADASTROPHE_CLI_SMOKE_KEEP_APP_DATA=1 to inspect it after the run");
    }
  } finally {
    if (!keepAppData && !process.env.CADASTROPHE_CLI_SMOKE_APP_DATA_DIR) {
      rmSync(appDataDir, { recursive: true, force: true });
    }
  }
}

function buildCliBins() {
  log("building Rust CLI bins");
  execFileSync("cargo", ["build", "--manifest-path", "src-tauri/Cargo.toml", "--bins"], {
    cwd: repoRoot,
    stdio: "inherit"
  });
}

function buildStructuralSidecar() {
  log("building structural sidecar fallback beside Rust CLI bins");
  execFileSync(process.execPath, ["scripts/build-structural-sidecar.mjs", "--profile", "debug"], {
    cwd: repoRoot,
    stdio: "inherit"
  });
  assert(existsSync(adjacentSidecarPath), `structural sidecar exists at ${adjacentSidecarPath}`);
}

function seedFixtureDatabase() {
  log("seeding app-data SQLite fixture");
  mkdirSync(appDataDir, { recursive: true });
  runCli("cadastrophe-session-current");
  const dbPath = join(appDataDir, "cadastrophe.sqlite3");
  const createdAt = new Date().toISOString();
  const seedSource = "// @main_component wall_bracket\ncube([1, 1, 1]);\n";
  const sql = `
PRAGMA foreign_keys = ON;
BEGIN;
INSERT INTO sessions (
  id, title, selected_runtime, status, active_revision_id,
  created_at, updated_at, last_viewed_at, connected_ui_clients,
  archived_at, deleted_at, metadata_json
) VALUES (
  ${q(sessionId)}, 'Manual CLI smoke', 'openscad-wasm', 'idle', NULL,
  ${q(createdAt)}, ${q(createdAt)}, ${q(createdAt)}, 0,
  NULL, NULL, NULL
);
INSERT INTO revisions (
  id, session_id, parent_revision_id, restored_from_revision_id,
  source_language, source, parameters_json, diagnostics_json,
  created_at, metadata_json
) VALUES (
  ${q(seedRevisionId)}, ${q(sessionId)}, NULL, NULL,
  'openscad', ${q(seedSource)}, '[]', '{"ok":true,"elapsedMs":0,"items":[]}',
  ${q(createdAt)}, '{"userEvents":[]}'
);
UPDATE sessions SET active_revision_id = ${q(seedRevisionId)} WHERE id = ${q(sessionId)};
INSERT INTO agent_runs (
  id, session_id, input_revision_id, output_revision_id, status, prompt,
  created_at, updated_at, started_at, completed_at, error, active_step,
  external_agent, external_thread_id, external_turn_id, metadata_json
) VALUES (
  ${q(runId)}, ${q(sessionId)}, ${q(seedRevisionId)}, NULL, 'queued', 'Manual CLI smoke run',
  ${q(createdAt)}, ${q(createdAt)}, NULL, NULL, NULL, NULL,
  'manual-cli-smoke', NULL, NULL, NULL
);
INSERT INTO agent_run_events (
  id, session_id, run_id, revision_id, event_type, sequence, created_at, payload_json, metadata_json
) VALUES (
  'manual-cli-run-created-event-1', ${q(sessionId)}, ${q(runId)}, ${q(seedRevisionId)},
  'agent.run.created', 1, ${q(createdAt)}, '{"status":"queued","prompt":"Manual CLI smoke run"}', NULL
);
COMMIT;
`;
  execFileSync("sqlite3", [dbPath], {
    cwd: repoRoot,
    input: sql,
    encoding: "utf8",
    stdio: ["pipe", "pipe", "inherit"]
  });
}

function runCli(name, ...args) {
  const result = spawnSync(binPath(name), ["--app-data-dir", appDataDir, ...args], {
    cwd: repoRoot,
    encoding: "utf8"
  });
  const output = result.status === 0 ? result.stdout : result.stderr;
  let envelope;
  try {
    envelope = JSON.parse(output);
  } catch (error) {
    throw new Error(`${name} did not emit JSON.\nstatus: ${result.status}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}\n${error}`);
  }
  if (result.status !== 0 || envelope.ok !== true) {
    throw new Error(`${name} failed.\nstatus: ${result.status}\n${JSON.stringify(envelope, null, 2)}`);
  }
  log(`ok ${name}`);
  return envelope;
}

function binPath(name) {
  const executable = process.platform === "win32" ? `${name}.exe` : name;
  return join(targetRoot, executable);
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
}

function q(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function requireString(value, label) {
  assert(typeof value === "string" && value.length > 0, `${label} is a non-empty string`);
  return value;
}

function assertEqual(actual, expected, label) {
  if (actual !== expected) {
    throw new Error(`${label}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assert(condition, label) {
  if (!condition) {
    throw new Error(label);
  }
}

function requireCommand(command) {
  const result = spawnSync(command, ["--version"], { encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw new Error(`Required command not available: ${command}`);
  }
}

function log(message) {
  console.log(`[manual-cli-smoke] ${message}`);
}
