#include "mesh_validator.h"

#include <open3d/Open3D.h>

#include <cmath>
#include <cstdint>
#include <iostream>
#include <random>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace local = cadastrophe::mesh;

namespace {

struct MeshInput {
    std::vector<local::Vec3> vertices;
    std::vector<local::Triangle> triangles;
};

open3d::geometry::TriangleMesh to_open3d(const MeshInput& input) {
    open3d::geometry::TriangleMesh mesh;
    for (const local::Vec3& vertex : input.vertices) {
        mesh.vertices_.emplace_back(vertex.x, vertex.y, vertex.z);
    }
    for (const local::Triangle& triangle : input.triangles) {
        mesh.triangles_.emplace_back(static_cast<int>(triangle[0]),
                                     static_cast<int>(triangle[1]),
                                     static_cast<int>(triangle[2]));
    }
    return mesh;
}

void expect_equal(bool actual, bool expected,
                  const std::string& case_name,
                  const std::string& property) {
    if (actual != expected) {
        throw std::runtime_error(case_name + ": " + property +
                                 " differs (local=" + (actual ? "true" : "false") +
                                 ", Open3D=" + (expected ? "true" : "false") + ")");
    }
}

void compare_all(const std::string& case_name,
                 const MeshInput& input,
                 bool compare_volume = false) {
    const local::TriangleMeshValidator local_mesh(input.vertices, input.triangles);
    open3d::geometry::TriangleMesh reference = to_open3d(input);
    expect_equal(local_mesh.IsEdgeManifold(true), reference.IsEdgeManifold(true),
                 case_name, "IsEdgeManifold(true)");
    expect_equal(local_mesh.IsEdgeManifold(false), reference.IsEdgeManifold(false),
                 case_name, "IsEdgeManifold(false)");
    expect_equal(local_mesh.IsVertexManifold(), reference.IsVertexManifold(),
                 case_name, "IsVertexManifold");
    expect_equal(local_mesh.IsSelfIntersecting(), reference.IsSelfIntersecting(),
                 case_name, "IsSelfIntersecting");
    expect_equal(local_mesh.IsOrientable(), reference.IsOrientable(),
                 case_name, "IsOrientable");
    expect_equal(local_mesh.IsWatertight(), reference.IsWatertight(),
                 case_name, "IsWatertight");
    if (compare_volume) {
        const double actual = local_mesh.GetVolume();
        const double expected = reference.GetVolume();
        const double tolerance = 1e-12 * std::max(1.0, std::abs(expected));
        if (std::abs(actual - expected) > tolerance) {
            throw std::runtime_error(case_name + ": GetVolume differs");
        }
    }
}

MeshInput box() {
    return {
        {{0, 0, 0}, {1, 0, 0}, {0, 0, 1}, {1, 0, 1},
         {0, 1, 0}, {1, 1, 0}, {0, 1, 1}, {1, 1, 1}},
        {{4, 7, 5}, {4, 6, 7}, {0, 2, 4}, {2, 6, 4},
         {0, 1, 2}, {1, 3, 2}, {1, 5, 7}, {1, 7, 3},
         {2, 3, 7}, {2, 7, 6}, {0, 4, 1}, {1, 4, 5}},
    };
}

void compare_degenerate_validator() {
    const std::vector<MeshInput> cases = {
        {{{0, 0, 0}, {1, 0, 0}, {0, 1, 0}}, {{0, 1, 2}}},
        {{{0, 0, 0}, {1, 0, 0}, {0, 1, 0}}, {{0, 0, 2}}},
        // Equal positions with different indices are not degenerate in Open3D.
        {{{0, 0, 0}, {0, 0, 0}, {0, 1, 0}}, {{0, 1, 2}}},
    };
    for (std::size_t index = 0; index < cases.size(); ++index) {
        const MeshInput& input = cases[index];
        const local::TriangleMeshValidator local_mesh(input.vertices, input.triangles);
        open3d::geometry::TriangleMesh reference = to_open3d(input);
        const std::size_t before = reference.triangles_.size();
        reference.RemoveDegenerateTriangles();
        const bool reference_has_degenerate = reference.triangles_.size() != before;
        expect_equal(local_mesh.HasDegenerateTriangles(), reference_has_degenerate,
                     "degenerate-" + std::to_string(index),
                     "HasDegenerateTriangles/RemoveDegenerateTriangles");
        if (local_mesh.triangles().size() != before) {
            throw std::runtime_error("HasDegenerateTriangles mutated the local mesh");
        }
    }
}

void compare_volume_failure(const MeshInput& input) {
    const local::TriangleMeshValidator local_mesh(input.vertices, input.triangles);
    open3d::geometry::TriangleMesh reference = to_open3d(input);
    bool local_threw = false;
    bool reference_threw = false;
    try {
        (void)local_mesh.GetVolume();
    } catch (const std::exception&) {
        local_threw = true;
    }
    try {
        (void)reference.GetVolume();
    } catch (const std::exception&) {
        reference_threw = true;
    }
    expect_equal(local_threw, reference_threw, "open-box", "GetVolume failure");
    if (!local_threw) {
        throw std::runtime_error("open-box: GetVolume unexpectedly succeeded");
    }
}

void compare_random_triangle_pairs() {
    std::mt19937_64 generator(0xcada5705ULL);
    std::uniform_real_distribution<double> coordinate(-2.0, 2.0);
    for (int case_index = 0; case_index < 1000; ++case_index) {
        MeshInput input;
        for (int vertex = 0; vertex < 6; ++vertex) {
            input.vertices.push_back(
                {coordinate(generator), coordinate(generator), coordinate(generator)});
        }
        input.triangles = {{0, 1, 2}, {3, 4, 5}};
        const local::TriangleMeshValidator local_mesh(input.vertices, input.triangles);
        const open3d::geometry::TriangleMesh reference = to_open3d(input);
        expect_equal(local_mesh.IsSelfIntersecting(), reference.IsSelfIntersecting(),
                     "random-triangle-pair-" + std::to_string(case_index),
                     "IsSelfIntersecting");
    }
}

void compare_random_indexed_meshes() {
    std::mt19937_64 generator(0x0f3d1234ULL);
    std::uniform_real_distribution<double> coordinate(-1.0, 1.0);
    std::uniform_int_distribution<int> vertex_index(0, 8);
    for (int case_index = 0; case_index < 250; ++case_index) {
        MeshInput input;
        for (int vertex = 0; vertex < 9; ++vertex) {
            input.vertices.push_back(
                {coordinate(generator), coordinate(generator), coordinate(generator)});
        }
        for (int triangle_index = 0; triangle_index < 8; ++triangle_index) {
            std::size_t a = static_cast<std::size_t>(vertex_index(generator));
            std::size_t b = static_cast<std::size_t>(vertex_index(generator));
            std::size_t c = static_cast<std::size_t>(vertex_index(generator));
            while (b == a) b = static_cast<std::size_t>(vertex_index(generator));
            while (c == a || c == b) c = static_cast<std::size_t>(vertex_index(generator));
            input.triangles.push_back({a, b, c});
        }
        compare_all("random-indexed-mesh-" + std::to_string(case_index), input);
    }
}

void compare_near_coplanar_pairs() {
    const std::vector<double> offsets = {
        0.0, 1e-12, -1e-12, 1e-9, -1e-9, 1e-6, -1e-6, 1e-3, -1e-3};
    for (double offset : offsets) {
        const MeshInput input{
            {{0, 0, 0}, {0, 1, 0}, {1, 0, 0},
             {0.1, 0.1, offset}, {0.1, 1.1, offset}, {1.1, 0.1, offset}},
            {{0, 1, 2}, {3, 4, 5}},
        };
        const local::TriangleMeshValidator local_mesh(input.vertices, input.triangles);
        const open3d::geometry::TriangleMesh reference = to_open3d(input);
        expect_equal(local_mesh.IsSelfIntersecting(), reference.IsSelfIntersecting(),
                     "near-coplanar-" + std::to_string(offset),
                     "IsSelfIntersecting");
    }
}

}  // namespace

int main() {
    try {
        compare_all("empty", {});
        compare_all("single-triangle",
                    {{{0, 0, 0}, {0, 0, 1}, {0, 1, 1}}, {{0, 1, 2}}});
        compare_all("non-manifold-edge",
                    {{{0, 0, 0}, {0, 0, 1}, {0, 1, 1}, {0, 0, 2}, {1, 0.5, 1}},
                     {{0, 1, 2}, {1, 2, 3}, {1, 2, 4}}});
        compare_all("non-manifold-vertex",
                    {{{0, 0, 0}, {1, 1, 1}, {1, 0, 1}, {0, 1, 1},
                      {1, 1, -1}, {1, 0, -1}},
                     {{0, 1, 2}, {0, 2, 3}, {0, 4, 5}}});
        compare_all("simple-intersection",
                    {{{0, 0, 0}, {0, 1, 0}, {1, 0, 0}, {1, 1, 0},
                      {0.5, 0.5, -1}, {0, 1, 1}, {1, 0, 1}},
                     {{0, 1, 2}, {1, 2, 3}, {4, 5, 6}}});
        compare_all("coplanar-intersection",
                    {{{0, 0, 0}, {0, 1, 0}, {1, 0, 0},
                      {0.1, 0.1, 0}, {0.1, 1.1, 0}, {1.1, 0.1, 0}},
                     {{0, 1, 2}, {3, 4, 5}}});
        compare_all("box", box(), true);
        MeshInput open_box = box();
        open_box.triangles.pop_back();
        compare_all("open-box", open_box);
        compare_volume_failure(open_box);
        compare_degenerate_validator();
        compare_random_triangle_pairs();
        compare_random_indexed_meshes();
        compare_near_coplanar_pairs();
        std::cout << "Open3D parity passed for fixed, randomized, and near-coplanar cases\n";
        return 0;
    } catch (const std::exception& error) {
        std::cerr << error.what() << '\n';
        return 1;
    }
}
