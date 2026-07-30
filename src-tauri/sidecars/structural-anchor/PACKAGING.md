# Structural Anchor Sidecar Packaging

`cadastrophe-structural-anchor` is the native C++ sidecar used by the Track C
Rust CLI wrapper. It reads a deterministic JSON input document from stdin or
`--input`, evaluates one final STL against the committed Plan and artifact
metadata with the repository-local STL parser, and writes
`cadastrophe.structural_report.v1` JSON to stdout.

## Build

Native STL parser build:

```sh
npm run build:sidecar
```

The npm script builds the sidecar and installs it beside the Rust CLI
executables at `src-tauri/target/debug/cadastrophe-structural-anchor`, which is
the default lookup path used by `cadastrophe-evaluate-structural` and
`cadastrophe-finalize`.

Equivalent manual CMake build:

```sh
cmake -S src-tauri/sidecars/structural-anchor \
  -B /tmp/cadastrophe-structural-anchor-build
cmake --build /tmp/cadastrophe-structural-anchor-build
```

The native path is deterministic and sufficient for contract tests.

## Runtime Input

Minimum input:

```json
{
  "runId": "run-1",
  "revisionId": "revision-1",
  "artifactId": "artifact-1",
  "planPath": "plan.json",
  "stlPath": "artifact.stl",
  "artifactManifest": {
    "id": "artifact-1",
    "bytes": 684,
    "sha256": "..."
  },
  "runtimeDiagnostics": {
    "ok": true,
    "elapsedMs": 12,
    "items": []
  },
  "sourceText": "// @main_component wall_bracket\ncube([1, 1, 1]);"
}
```

Relative `*Path` values are resolved from the input file directory when
`--input` is used, otherwise from the current working directory.

## Platform Checklist

macOS:

- Build universal or per-architecture sidecars matching the Tauri bundle.
- Copy the executable into the Tauri sidecar resources path with executable
  mode preserved.
- Set install names/rpaths with `@executable_path` or `@loader_path` and verify
  with `otool -L cadastrophe-structural-anchor`.
- Sign the sidecar with the app identity before notarization.
- Verify Gatekeeper launch from a quarantined packaged `.app`.

Windows:

- Build with the same MSVC runtime family as the Tauri release build.
- Copy `cadastrophe-structural-anchor.exe` into the sidecar directory.
- Verify executable discovery from the packaged app, not only from a developer shell.
- Sign the `.exe` before installer packaging.
- Run a fixture evaluation from the installed app directory on a clean VM.

Linux:

- Build on the oldest supported glibc baseline or ship a compatible runtime
  container.
- Verify `ldd cadastrophe-structural-anchor` has no unresolved C++ runtime
  libraries.
- Keep executable permissions when building AppImage, deb, and rpm artifacts.
- Run the deterministic fixture evaluation from the installed bundle path.

Release validation:

- Run `npm test -- tests/structural-anchor.test.ts` against the native build.
- Confirm stdout is valid JSON and stderr is empty for passing fixture runs.
