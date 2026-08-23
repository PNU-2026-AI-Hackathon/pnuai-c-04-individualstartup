import { copyFileSync, existsSync, mkdirSync, chmodSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

export function buildSidecars({
  repoRoot,
  profile,
  targetRoot,
  buildDir,
  targetTriple,
  sidecars
}) {
  requireCommand("cmake");
  for (const sidecar of sidecars) {
    buildSidecar({
      repoRoot,
      profile,
      targetRoot,
      buildDir,
      targetTriple,
      sidecar
    });
  }
}

export function inferTargetTriple() {
  const explicit = process.env.CARGO_BUILD_TARGET
    ?? process.env.TARGET
    ?? process.env.TAURI_ENV_TARGET_TRIPLE;
  if (explicit) {
    return explicit;
  }
  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw new Error("Unable to infer target triple: rustc -vV failed");
  }
  const hostLine = result.stdout
    .split(/\r?\n/)
    .find((line) => line.startsWith("host: "));
  if (!hostLine) {
    throw new Error("Unable to infer target triple: rustc -vV did not report host");
  }
  return hostLine.slice("host: ".length).trim();
}

function buildSidecar({
  repoRoot,
  profile,
  targetRoot,
  buildDir,
  targetTriple,
  sidecar
}) {
  const cmakeOptions = sidecar.cmakeOptions ?? [];
  const cmakeConfiguration = profile === "release" ? "Release" : "Debug";
  const sidecarRoot = join(repoRoot, "src-tauri", "sidecars", sidecar.name);
  const buildRoot = resolve(
    repoRoot,
    buildDir
      ? join(buildDir, sidecar.name)
      : join("src-tauri", "target", sidecar.name, profile)
  );
  const executableName = process.platform === "win32"
    ? `${sidecar.executable}.exe`
    : sidecar.executable;
  const bundleExecutableName = process.platform === "win32"
    ? `${sidecar.executable}-${targetTriple}.exe`
    : `${sidecar.executable}-${targetTriple}`;
  const builtExecutable = join(buildRoot, executableName);
  const adjacentExecutable = join(targetRoot, profile, executableName);
  const bundleExecutable = join(targetRoot, profile, bundleExecutableName);

  mkdirSync(buildRoot, { recursive: true });
  mkdirSync(dirname(adjacentExecutable), { recursive: true });

  run(repoRoot, "cmake", [
    "-S",
    sidecarRoot,
    "-B",
    buildRoot,
    ...cmakeOptions,
    `-DCMAKE_BUILD_TYPE=${cmakeConfiguration}`,
    `-DCMAKE_RUNTIME_OUTPUT_DIRECTORY=${buildRoot}`,
    `-DCMAKE_RUNTIME_OUTPUT_DIRECTORY_${cmakeConfiguration.toUpperCase()}=${buildRoot}`
  ]);
  run(repoRoot, "cmake", [
    "--build",
    buildRoot,
    "--config",
    cmakeConfiguration
  ]);

  if (!existsSync(builtExecutable)) {
    throw new Error(`${sidecar.name} sidecar build did not produce ${builtExecutable}`);
  }
  copyFileSync(builtExecutable, adjacentExecutable);
  copyFileSync(builtExecutable, bundleExecutable);
  if (process.platform !== "win32") {
    chmodSync(adjacentExecutable, 0o755);
    chmodSync(bundleExecutable, 0o755);
  }
  console.log(`[build-sidecar] installed ${adjacentExecutable}`);
  console.log(`[build-sidecar] installed ${bundleExecutable}`);
}

function requireCommand(command) {
  const result = spawnSync(command, ["--version"], { encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw new Error(`Required command not available: ${command}`);
  }
}

function run(cwd, command, commandArgs) {
  const result = spawnSync(command, commandArgs, {
    cwd,
    stdio: "inherit"
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${commandArgs.join(" ")} failed with status ${result.status}`);
  }
}
