#pragma once

#include <array>
#include <cstddef>
#include <vector>

namespace cadastrophe::mesh {

struct Vec3 {
    double x = 0.0;
    double y = 0.0;
    double z = 0.0;
};

using Triangle = std::array<std::size_t, 3>;

/// A minimal, dependency-free triangle-mesh validator whose topology semantics
/// mirror Open3D's legacy geometry::TriangleMesh. Self-intersection uses the
/// tolerance-consistent predicate in mesh_intersection_fixed.h.
class TriangleMeshValidator {
public:
    TriangleMeshValidator(std::vector<Vec3> vertices,
                          std::vector<Triangle> triangles);

    bool IsEdgeManifold(bool allow_boundary_edges = true) const;
    bool IsVertexManifold() const;
    bool IsSelfIntersecting() const;
    bool IsOrientable() const;
    bool IsWatertight() const;
    bool HasDegenerateTriangles() const;

    /// Merges vertices within eps of the first unmerged vertex in input order,
    /// averages their positions, and remaps triangle indices.
    TriangleMeshValidator& MergeCloseVertices(double eps = 1e-6);

    /// Removes triangles containing the same vertex index more than once.
    TriangleMeshValidator& RemoveDegenerateTriangles();

    /// Removes vertices not referenced by any triangle and compacts indices.
    TriangleMeshValidator& RemoveUnreferencedVertices();

    /// Returns the absolute signed-tetrahedra volume. As in Open3D, volume is
    /// only defined for watertight, orientable meshes.
    double GetVolume() const;

    const std::vector<Vec3>& vertices() const { return vertices_; }
    const std::vector<Triangle>& triangles() const { return triangles_; }

private:
    std::vector<Vec3> vertices_;
    std::vector<Triangle> triangles_;
};

}  // namespace cadastrophe::mesh
