# Independent mesh validator

`cadastrophe-mesh-validator` is a minimal C++17 implementation of the mesh
validation behavior used by Open3D 0.19.0's legacy `TriangleMesh`. Production
code neither includes nor links Open3D.

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
- `MergeCloseVertices(eps)` follows Open3D's input-order grouping, averages
  each representative and its still-unmapped radius neighbors, and remaps all
  triangle indices. Mesh loading uses an epsilon of `1e-6`.
- `RemoveDegenerateTriangles()` compacts the triangle array using Open3D's
  repeated-index predicate.
- `RemoveUnreferencedVertices()` preserves referenced vertex order while
  compacting the vertex array and remapping triangle indices.
- `GetVolume()` enforces watertightness and orientability, then returns the
  absolute sum of signed origin-tetrahedron volumes.
