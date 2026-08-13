# Independent mesh validator

`cadastrophe-mesh-validator` is a minimal C++17 implementation of the mesh
validation behavior used by Open3D 0.19.0's legacy `TriangleMesh`. Production
code neither includes nor links Open3D. The local `Open3D/` directory is ignored
and is used only as the reference implementation for parity tests.

The traced behavior is:

- `IsEdgeManifold(bool)` builds the ordered-edge-to-triangle map and applies
  Open3D's one-or-two / exactly-two adjacency rules.
- `IsVertexManifold()` builds each vertex star's opposite-edge graph and checks
  it with breadth-first traversal.
- `IsSelfIntersecting()` skips triangle pairs sharing an index, applies the
  inclusive AABB test, normalizes each six-vertex pair, and runs the
  Tomas Möller triangle intersection test with Open3D's epsilon.
- `IsOrientable()` mirrors `OrientTriangleHelper`, including its edge hash and
  unordered traversal behavior.
- `IsWatertight()` is exactly the edge-closed, vertex-manifold, and
  non-self-intersecting conjunction. Orientability is intentionally separate.
- `HasDegenerateTriangles()` uses the same repeated-index predicate as
  `RemoveDegenerateTriangles()` but never mutates the mesh.
- `GetVolume()` enforces watertightness and orientability, then returns the
  absolute sum of signed origin-tetrahedron volumes.

Run the Open3D-linked test-only comparison with:

```sh
npm run test:mesh-parity
```

It requires `Open3DConfig.cmake`, by default under
`Open3D/install/lib/cmake/Open3D`. Set `OPEN3D_DIR` to use a different local
Open3D install. Missing reference builds fail the command immediately.

The parity binary compares fixed manifold/non-manifold, boundary,
self-intersection, coplanar, degenerate, watertight, and volume cases, plus
deterministically randomized triangle pairs and indexed meshes.
