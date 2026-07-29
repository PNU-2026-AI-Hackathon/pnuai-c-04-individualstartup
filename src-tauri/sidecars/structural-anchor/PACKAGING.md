# Structural Anchor Sidecar Packaging

`cadastrophe-structural-anchor` is the native C++ sidecar used by the Track C
Rust CLI wrapper. It reads a deterministic JSON input document from stdin or
`--input`, evaluates one final STL against the committed Plan and artifact
metadata, and writes `cadastrophe.structural_report.v1` JSON to stdout.

## Build

Fallback STL parser build, used by fixture tests and developer machines without
Open3D:

```sh
npm run build:sidecar
```

The npm script builds the fallback sidecar and installs it beside the Rust CLI
executables at `src-tauri/target/debug/cadastrophe-structural-anchor`, which is
the default lookup path used by `cadastrophe-evaluate-structural` and
`cadastrophe-finalize`.

Equivalent manual CMake build:

```sh
cmake -S src-tauri/sidecars/structural-anchor \
  -B /tmp/cadastrophe-structural-anchor-build \
  -DCADASTROPHE_STRUCTURAL_ANCHOR_USE_OPEN3D=OFF
cmake --build /tmp/cadastrophe-structural-anchor-build
```

Open3D build:

```sh
cmake -S src-tauri/sidecars/structural-anchor \
  -B /tmp/cadastrophe-structural-anchor-open3d-build \
  -DCADASTROPHE_STRUCTURAL_ANCHOR_REQUIRE_OPEN3D=ON \
  -DOpen3D_DIR=/absolute/path/to/Open3D/lib/cmake/Open3D
cmake --build /tmp/cadastrophe-structural-anchor-open3d-build --config Release
```

The fallback path is deterministic and sufficient for contract tests. Release
builds should prefer Open3D so `is_self_intersecting`, `get_volume`,
`is_edge_manifold`, `is_vertex_manifold`, and `is_orientable` come from Open3D.

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
- Bundle Open3D `.dylib` dependencies beside the sidecar or in `Frameworks`.
- Set install names/rpaths with `@executable_path` or `@loader_path` and verify
  with `otool -L cadastrophe-structural-anchor`.
- Sign the sidecar and every bundled Open3D dynamic library with the app
  identity before notarization.
- Verify Gatekeeper launch from a quarantined packaged `.app`.

Windows:

- Build with the same MSVC runtime family as the Tauri release build.
- Copy `cadastrophe-structural-anchor.exe` and required Open3D `.dll` files into
  the sidecar directory.
- Verify DLL discovery from the packaged app, not only from a developer shell.
- Sign the `.exe` and bundled `.dll` files before installer packaging.
- Run a fixture evaluation from the installed app directory on a clean VM.

Linux:

- Build on the oldest supported glibc baseline or ship a compatible runtime
  container.
- Bundle Open3D `.so` dependencies or set an app-local rpath such as
  `$ORIGIN/lib`.
- Verify `ldd cadastrophe-structural-anchor` has no unresolved Open3D or C++
  runtime libraries.
- Keep executable permissions when building AppImage, deb, and rpm artifacts.
- Run the deterministic fixture evaluation from the installed bundle path.

Release validation:

- Run `npm test -- tests/structural-anchor.test.ts` against the fallback build.
- Run the same fixture input against the Open3D build and inspect only expected
  engine/topology detail differences.
- Confirm stdout is valid JSON and stderr is empty for passing fixture runs.
