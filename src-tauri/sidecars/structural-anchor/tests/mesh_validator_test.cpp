#include "mesh_validator.h"

#include <cmath>
#include <cstdlib>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

using cadastrophe::mesh::Triangle;
using cadastrophe::mesh::TriangleMeshValidator;
using cadastrophe::mesh::Vec3;

void require(bool condition, const std::string& message) {
    if (!condition) {
        std::cerr << message << '\n';
        std::exit(1);
    }
}

void test_merge_close_vertices() {
    TriangleMeshValidator mesh(
        {{0.0, 0.0, 0.0}, {0.75e-6, 0.0, 0.0}, {1.5e-6, 0.0, 0.0},
         {0.0, 1.0, 0.0}},
        {{0, 1, 3}, {1, 2, 3}});

    TriangleMeshValidator& result = mesh.MergeCloseVertices(1e-6);

    require(&result == &mesh, "MergeCloseVertices must return the mesh");
    require(mesh.vertices().size() == 3, "two close vertices must be merged");
    require(std::abs(mesh.vertices()[0].x - 0.375e-6) < 1e-18,
            "merged vertex must be the group average");
    require(mesh.triangles()[0] == Triangle{0, 0, 2},
            "merged triangle indices must be remapped");
    require(mesh.triangles()[1] == Triangle{0, 1, 2},
            "merge grouping must be based on the first unmerged vertex");
}

void test_remove_degenerate_triangles() {
    TriangleMeshValidator mesh(
        {{0.0, 0.0, 0.0}, {1.0, 0.0, 0.0}, {0.0, 1.0, 0.0}},
        {{0, 0, 1}, {0, 1, 2}, {2, 1, 2}});

    mesh.RemoveDegenerateTriangles();

    require(mesh.triangles().size() == 1,
            "all repeated-index triangles must be removed");
    require(mesh.triangles()[0] == Triangle{0, 1, 2},
            "non-degenerate triangle order must be preserved");
    require(!mesh.HasDegenerateTriangles(),
            "removed mesh must not report index-degenerate triangles");
}

void test_remove_unreferenced_vertices() {
    TriangleMeshValidator mesh(
        {{9.0, 9.0, 9.0}, {0.0, 0.0, 0.0}, {8.0, 8.0, 8.0},
         {1.0, 0.0, 0.0}, {0.0, 1.0, 0.0}},
        {{1, 3, 4}});

    mesh.RemoveUnreferencedVertices();

    require(mesh.vertices().size() == 3,
            "unreferenced vertices must be removed");
    require(mesh.vertices()[0].x == 0.0 && mesh.vertices()[1].x == 1.0,
            "referenced vertex order must be preserved");
    require(mesh.triangles()[0] == Triangle{0, 1, 2},
            "compacted vertex indices must be remapped");
}

void test_cleanup_sequence() {
    TriangleMeshValidator mesh(
        {{0.0, 0.0, 0.0}, {0.5e-6, 0.0, 0.0}, {1.0, 0.0, 0.0},
         {0.0, 1.0, 0.0}, {10.0, 10.0, 10.0}},
        {{0, 1, 2}, {0, 2, 3}});

    mesh.MergeCloseVertices().RemoveDegenerateTriangles().RemoveUnreferencedVertices();

    require(mesh.triangles().size() == 1,
            "cleanup must remove degeneracy introduced by vertex merging");
    require(mesh.vertices().size() == 3,
            "cleanup must remove vertices orphaned by triangle removal");
    require(mesh.triangles()[0] == Triangle{0, 1, 2},
            "cleanup sequence must leave compact valid indices");
}

void test_merge_radius_is_open_and_epsilon_fails_fast() {
    TriangleMeshValidator boundary_mesh(
        {{0.0, 0.0, 0.0}, {1e-6, 0.0, 0.0}, {0.0, 1.0, 0.0}},
        {{0, 1, 2}});
    boundary_mesh.MergeCloseVertices(1e-6);
    require(boundary_mesh.vertices().size() == 3,
            "Open3D radius search must exclude the exact radius boundary");

    bool threw = false;
    try {
        boundary_mesh.MergeCloseVertices(0.0);
    } catch (const std::invalid_argument&) {
        threw = true;
    }
    require(threw, "non-positive merge epsilon must fail fast");
}

}  // namespace

int main() {
    test_merge_close_vertices();
    test_remove_degenerate_triangles();
    test_remove_unreferenced_vertices();
    test_cleanup_sequence();
    test_merge_radius_is_open_and_epsilon_fails_fast();
    return 0;
}
