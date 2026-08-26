#pragma once

#include "mesh_validator.h"

#include <array>
#include <cstddef>
#include <vector>

namespace cadgen_ax::mesh::fixed {

struct IntersectionTolerance {
    // Signed distances are measured after the same pair-wise axis normalization
    // used by the legacy validator and after normalizing the plane normal.
    double plane_distance = 1e-6;

    // For unit normals, |n0 x n1| is sin(theta). This is therefore a relative
    // angular test rather than an absolute comparison of unnormalized normals.
    double normal_sine = 1e-6;
};

using TrianglePair = std::array<std::size_t, 2>;

/// Active replacement for the legacy triangle/triangle predicate.
///
/// TriangleMeshValidator::IsSelfIntersecting delegates to this implementation.
/// It uses a scale-aware normal comparison and represents an all-zero
/// triangle/plane slice as Coplanar instead of manufacturing a two-point
/// interval.
bool TriangleTriangleIntersects(const Vec3& p0,
                                const Vec3& p1,
                                const Vec3& p2,
                                const Vec3& q0,
                                const Vec3& q1,
                                const Vec3& q2,
                                const IntersectionTolerance& tolerance = {});

/// Returns every non-neighbor triangle pair accepted by the fixed predicate.
std::vector<TrianglePair> GetSelfIntersectingTriangles(
        const std::vector<Vec3>& vertices,
        const std::vector<Triangle>& triangles,
        const IntersectionTolerance& tolerance = {});

bool IsSelfIntersecting(const std::vector<Vec3>& vertices,
                        const std::vector<Triangle>& triangles,
                        const IntersectionTolerance& tolerance = {});

}  // namespace cadgen_ax::mesh::fixed
