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

/// A minimal, dependency-free triangle-mesh validator whose topology and
/// intersection semantics mirror Open3D's legacy geometry::TriangleMesh.
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
