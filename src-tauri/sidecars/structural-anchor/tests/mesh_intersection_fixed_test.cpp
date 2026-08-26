#include "mesh_intersection_fixed.h"
#include "mesh_validator.h"

#include <cmath>
#include <cstdlib>
#include <iostream>
#include <limits>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

using cadgen_ax::mesh::TriangleMeshValidator;
using cadgen_ax::mesh::Vec3;
using cadgen_ax::mesh::Triangle;
namespace fixed = cadgen_ax::mesh::fixed;

void require(bool condition, const std::string& message) {
    if (!condition) {
        std::cerr << message << '\n';
        std::exit(1);
    }
}

Vec3 rotate_z_45(const Vec3& point) {
    const double value = std::sqrt(0.5);
    return {value * point.x - value * point.y,
            value * point.x + value * point.y,
            point.z};
}

void test_vent_0_minimal_false_positive() {
    const Vec3 p0{4, 6.907261063532883, 19.302275553302263};
    const Vec3 p1{10, 6.907261063532883, 19.302275553302263};
    const Vec3 p2{10, 6.982303701729662, 19.354820974254790};
    const Vec3 q0{4, 7.150000000000000, 19.472243186433545};
    const Vec3 q1{124.4, 6.982303701729662, 19.354820974254790};
    const Vec3 q2{124.4, 7.150000000000000, 19.472243186433545};

    TriangleMeshValidator production({p0, p1, p2, q0, q1, q2},
                                     {{0, 1, 2}, {3, 4, 5}});
    require(!production.IsSelfIntersecting(),
            "the production validator must reject the vent_0 false positive");
    require(!fixed::TriangleTriangleIntersects(p0, p1, p2, q0, q1, q2),
            "the fixed predicate must reject the vent_0 false positive");

    require(!fixed::TriangleTriangleIntersects(p0, p1, p2, q1, q2, q0) &&
            !fixed::TriangleTriangleIntersects(p0, p1, p2, q2, q0, q1),
            "cyclic permutations must preserve the fixed result");
    require(!fixed::TriangleTriangleIntersects(
                rotate_z_45(p0), rotate_z_45(p1), rotate_z_45(p2),
                rotate_z_45(q0), rotate_z_45(q1), rotate_z_45(q2)),
            "rigid rotation must preserve the fixed result");
}

void test_coplanar_cases() {
    const Vec3 p0{0.0, 0.0, 0.0};
    const Vec3 p1{2.0, 0.0, 0.0};
    const Vec3 p2{0.0, 2.0, 0.0};

    require(fixed::TriangleTriangleIntersects(
                p0, p1, p2,
                {0.25, 0.25, 0.0}, {1.0, 0.25, 0.0}, {0.25, 1.0, 0.0}),
            "overlapping coplanar triangles must intersect");
    require(!fixed::TriangleTriangleIntersects(
                p0, p1, p2,
                {3.0, 3.0, 0.0}, {4.0, 3.0, 0.0}, {3.0, 4.0, 0.0}),
            "separated coplanar triangles must not intersect");
    require(fixed::TriangleTriangleIntersects(
                p0, p1, p2,
                {2.0, 0.0, 0.0}, {3.0, 0.0, 0.0}, {2.0, 1.0, 0.0}),
            "coplanar point contact must intersect");
}

void test_noncoplanar_cases() {
    const Vec3 p0{0.0, 0.0, 0.0};
    const Vec3 p1{2.0, 0.0, 0.0};
    const Vec3 p2{0.0, 2.0, 0.0};

    require(fixed::TriangleTriangleIntersects(
                p0, p1, p2,
                {0.5, 0.5, -1.0}, {0.5, 0.5, 1.0}, {1.0, 1.0, 1.0}),
            "a non-coplanar triangle piercing another triangle must intersect");
    require(!fixed::TriangleTriangleIntersects(
                p0, p1, p2,
                {0.0, 0.0, 3.0}, {2.0, 0.0, 3.0}, {0.0, 2.0, 3.0}),
            "parallel triangles on distinct planes must not intersect");
    require(fixed::TriangleTriangleIntersects(
                p0, p1, p2,
                {0.0, 0.0, 0.0}, {0.0, 0.0, 1.0}, {1.0, 0.0, 1.0}),
            "non-coplanar point contact must intersect");
}

void test_mesh_pair_enumeration() {
    const std::vector<Vec3> vertices{
        {0.0, 0.0, 0.0}, {2.0, 0.0, 0.0}, {0.0, 2.0, 0.0},
        {0.5, 0.5, -1.0}, {0.5, 0.5, 1.0}, {1.0, 1.0, 1.0}};
    const std::vector<Triangle> triangles{{0, 1, 2}, {3, 4, 5}};
    const auto pairs = fixed::GetSelfIntersectingTriangles(vertices, triangles);
    require(pairs.size() == 1 && pairs[0] == fixed::TrianglePair{0, 1},
            "mesh enumeration must return the intersecting non-neighbor pair");

    const std::vector<Vec3> neighbor_vertices{
        {0.0, 0.0, 0.0}, {1.0, 0.0, 0.0}, {0.0, 1.0, 0.0},
        {0.0, 0.0, 1.0}, {1.0, 0.0, 1.0}};
    const std::vector<Triangle> neighbors{{0, 1, 2}, {0, 3, 4}};
    require(fixed::GetSelfIntersectingTriangles(neighbor_vertices, neighbors).empty(),
            "mesh enumeration must preserve the shared-index neighbor exclusion");
}

void test_fail_fast_inputs() {
    bool threw = false;
    try {
        fixed::TriangleTriangleIntersects(
            {0.0, 0.0, 0.0}, {1.0, 0.0, 0.0}, {2.0, 0.0, 0.0},
            {0.0, 0.0, 1.0}, {1.0, 0.0, 1.0}, {0.0, 1.0, 1.0});
    } catch (const std::domain_error&) {
        threw = true;
    }
    require(threw, "a zero-area triangle must fail fast");

    threw = false;
    try {
        fixed::IntersectionTolerance invalid;
        invalid.normal_sine = std::numeric_limits<double>::quiet_NaN();
        fixed::TriangleTriangleIntersects(
            {0.0, 0.0, 0.0}, {1.0, 0.0, 0.0}, {0.0, 1.0, 0.0},
            {0.0, 0.0, 1.0}, {1.0, 0.0, 1.0}, {0.0, 1.0, 1.0}, invalid);
    } catch (const std::invalid_argument&) {
        threw = true;
    }
    require(threw, "an invalid tolerance must fail fast");

    threw = false;
    try {
        fixed::GetSelfIntersectingTriangles(
            {{0.0, 0.0, 0.0}, {1.0, 0.0, 0.0}, {0.0, 1.0, 0.0}},
            {{0, 1, 3}});
    } catch (const std::invalid_argument&) {
        threw = true;
    }
    require(threw, "an out-of-range mesh index must fail fast");
}

}  // namespace

int main() {
    test_vent_0_minimal_false_positive();
    test_coplanar_cases();
    test_noncoplanar_cases();
    test_mesh_pair_enumeration();
    test_fail_fast_inputs();
    return 0;
}
