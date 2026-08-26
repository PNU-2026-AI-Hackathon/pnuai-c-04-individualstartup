#include "mesh_validator.h"
#include "mesh_intersection_fixed.h"

// Topology behavior is independently implemented from Open3D 0.19.0's
// TriangleMesh validation methods (MIT). The active triangle/triangle
// intersection implementation is in mesh_intersection_fixed.cpp.

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <limits>
#include <map>
#include <queue>
#include <set>
#include <stdexcept>
#include <unordered_map>
#include <unordered_set>
#include <utility>

namespace cadgen_ax::mesh {
namespace {

using Edge = std::pair<std::size_t, std::size_t>;

struct EdgeHash {
    std::size_t operator()(const Edge& edge) const {
        // Matches Open3D utility::hash_eigen<Eigen::Vector2i>.
        std::size_t seed = 0;
        for (std::size_t value : {edge.first, edge.second}) {
            seed ^= std::hash<int>{}(static_cast<int>(value)) + 0x9e3779b9 +
                    (seed << 6) + (seed >> 2);
        }
        return seed;
    }
};

struct TriangleIndexHash {
    std::size_t operator()(std::size_t value) const {
        return std::hash<int>{}(static_cast<int>(value));
    }
};

constexpr std::size_t kNoIndex = std::numeric_limits<std::size_t>::max();

struct KdNode {
    std::size_t vertex_index = 0;
    std::size_t left = kNoIndex;
    std::size_t right = kNoIndex;
    int axis = 0;
};

Vec3 operator+(const Vec3& a, const Vec3& b) {
    return {a.x + b.x, a.y + b.y, a.z + b.z};
}

Vec3 operator-(const Vec3& a, const Vec3& b) {
    return {a.x - b.x, a.y - b.y, a.z - b.z};
}

Vec3 operator/(const Vec3& v, double divisor) {
    return {v.x / divisor, v.y / divisor, v.z / divisor};
}

Vec3 cross(const Vec3& a, const Vec3& b) {
    return {
        a.y * b.z - a.z * b.y,
        a.z * b.x - a.x * b.z,
        a.x * b.y - a.y * b.x,
    };
}

double dot(const Vec3& a, const Vec3& b) {
    return a.x * b.x + a.y * b.y + a.z * b.z;
}

double component(const Vec3& v, int axis) {
    if (axis == 0) return v.x;
    if (axis == 1) return v.y;
    return v.z;
}

std::size_t build_kd_tree(std::vector<std::size_t>& indices,
                          std::size_t begin,
                          std::size_t end,
                          int depth,
                          const std::vector<Vec3>& vertices,
                          std::vector<KdNode>& nodes) {
    if (begin == end) return kNoIndex;

    const int axis = depth % 3;
    const std::size_t middle = begin + (end - begin) / 2;
    std::nth_element(
        indices.begin() + static_cast<std::ptrdiff_t>(begin),
        indices.begin() + static_cast<std::ptrdiff_t>(middle),
        indices.begin() + static_cast<std::ptrdiff_t>(end),
        [&](std::size_t left, std::size_t right) {
            const double left_value = component(vertices[left], axis);
            const double right_value = component(vertices[right], axis);
            return left_value != right_value ? left_value < right_value : left < right;
        });

    const std::size_t node_index = nodes.size();
    nodes.push_back({indices[middle], kNoIndex, kNoIndex, axis});
    const std::size_t left =
        build_kd_tree(indices, begin, middle, depth + 1, vertices, nodes);
    const std::size_t right =
        build_kd_tree(indices, middle + 1, end, depth + 1, vertices, nodes);
    nodes[node_index].left = left;
    nodes[node_index].right = right;
    return node_index;
}

void radius_search(const std::vector<KdNode>& nodes,
                   std::size_t node_index,
                   const std::vector<Vec3>& vertices,
                   const Vec3& target,
                   double radius_squared,
                   std::vector<std::size_t>& neighbors) {
    if (node_index == kNoIndex) return;

    const KdNode& node = nodes[node_index];
    const Vec3& candidate = vertices[node.vertex_index];
    const double dx = candidate.x - target.x;
    const double dy = candidate.y - target.y;
    const double dz = candidate.z - target.z;
    if (dx * dx + dy * dy + dz * dz < radius_squared) {
        neighbors.push_back(node.vertex_index);
    }

    const double delta = component(target, node.axis) - component(candidate, node.axis);
    const std::size_t near_child = delta < 0.0 ? node.left : node.right;
    const std::size_t far_child = delta < 0.0 ? node.right : node.left;
    radius_search(nodes, near_child, vertices, target, radius_squared, neighbors);
    if (delta * delta < radius_squared) {
        radius_search(nodes, far_child, vertices, target, radius_squared, neighbors);
    }
}

Edge ordered_edge(std::size_t a, std::size_t b) {
    return a < b ? Edge{a, b} : Edge{b, a};
}

std::map<Edge, std::vector<std::size_t>> edge_to_triangles(
        const std::vector<Triangle>& triangles) {
    std::map<Edge, std::vector<std::size_t>> result;
    for (std::size_t index = 0; index < triangles.size(); ++index) {
        const Triangle& triangle = triangles[index];
        result[ordered_edge(triangle[0], triangle[1])].push_back(index);
        result[ordered_edge(triangle[1], triangle[2])].push_back(index);
        result[ordered_edge(triangle[2], triangle[0])].push_back(index);
    }
    return result;
}

bool aabb_intersects(const Vec3& min0,
                     const Vec3& max0,
                     const Vec3& min1,
                     const Vec3& max1) {
    return !(max0.x < min1.x || min0.x > max1.x ||
             max0.y < min1.y || min0.y > max1.y ||
             max0.z < min1.z || min0.z > max1.z);
}

std::pair<Vec3, Vec3> triangle_bounds(const Vec3& a,
                                      const Vec3& b,
                                      const Vec3& c) {
    return {
        {std::min({a.x, b.x, c.x}), std::min({a.y, b.y, c.y}),
         std::min({a.z, b.z, c.z})},
        {std::max({a.x, b.x, c.x}), std::max({a.y, b.y, c.y}),
         std::max({a.z, b.z, c.z})},
    };
}

double orient2d(double ax, double ay,
                double bx, double by,
                double cx, double cy) {
    return (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
}

bool point_in_triangle_2d(double px, double py,
                          const Vec3& a, const Vec3& b, const Vec3& c,
                          int axis0, int axis1) {
    const double ab = orient2d(component(a, axis0), component(a, axis1),
                               component(b, axis0), component(b, axis1), px, py);
    const double bc = orient2d(component(b, axis0), component(b, axis1),
                               component(c, axis0), component(c, axis1), px, py);
    const double ca = orient2d(component(c, axis0), component(c, axis1),
                               component(a, axis0), component(a, axis1), px, py);
    return (ab >= 0.0 && bc >= 0.0 && ca >= 0.0) ||
           (ab <= 0.0 && bc <= 0.0 && ca <= 0.0);
}

bool segments_intersect_2d(const Vec3& a, const Vec3& b,
                           const Vec3& c, const Vec3& d,
                           int axis0, int axis1) {
    const double ax = component(a, axis0);
    const double ay = component(a, axis1);
    const double bx = component(b, axis0);
    const double by = component(b, axis1);
    const double cx = component(c, axis0);
    const double cy = component(c, axis1);
    const double dx = component(d, axis0);
    const double dy = component(d, axis1);
    const double o1 = orient2d(ax, ay, bx, by, cx, cy);
    const double o2 = orient2d(ax, ay, bx, by, dx, dy);
    const double o3 = orient2d(cx, cy, dx, dy, ax, ay);
    const double o4 = orient2d(cx, cy, dx, dy, bx, by);
    const auto within = [](double value, double end0, double end1) {
        return value >= std::min(end0, end1) && value <= std::max(end0, end1);
    };
    if (o1 == 0.0 && within(cx, ax, bx) && within(cy, ay, by)) return true;
    if (o2 == 0.0 && within(dx, ax, bx) && within(dy, ay, by)) return true;
    if (o3 == 0.0 && within(ax, cx, dx) && within(ay, cy, dy)) return true;
    if (o4 == 0.0 && within(bx, cx, dx) && within(by, cy, dy)) return true;
    return ((o1 > 0.0) != (o2 > 0.0)) && ((o3 > 0.0) != (o4 > 0.0));
}

bool coplanar_triangles_intersect(const Vec3& normal,
                                  const Vec3& p0, const Vec3& p1, const Vec3& p2,
                                  const Vec3& q0, const Vec3& q1, const Vec3& q2) {
    const Vec3 absolute{std::abs(normal.x), std::abs(normal.y), std::abs(normal.z)};
    int axis0 = 0;
    int axis1 = 1;
    if (absolute.x > absolute.y) {
        if (absolute.x > absolute.z) {
            axis0 = 1;
            axis1 = 2;
        }
    } else if (absolute.z <= absolute.y) {
        axis0 = 0;
        axis1 = 2;
    }
    const std::array<std::pair<Vec3, Vec3>, 3> p_edges = {
        std::pair{p0, p1}, std::pair{p1, p2}, std::pair{p2, p0}};
    const std::array<std::pair<Vec3, Vec3>, 3> q_edges = {
        std::pair{q0, q1}, std::pair{q1, q2}, std::pair{q2, q0}};
    for (const auto& p_edge : p_edges) {
        for (const auto& q_edge : q_edges) {
            if (segments_intersect_2d(p_edge.first, p_edge.second,
                                      q_edge.first, q_edge.second, axis0, axis1)) {
                return true;
            }
        }
    }
    return point_in_triangle_2d(component(p0, axis0), component(p0, axis1),
                                q0, q1, q2, axis0, axis1) ||
           point_in_triangle_2d(component(q0, axis0), component(q0, axis1),
                                p0, p1, p2, axis0, axis1);
}

struct PlaneDistances {
    std::array<double, 3> values;
    bool all_positive_or_negative = false;
};

PlaneDistances distances_to_plane(const Vec3& normal,
                                  const Vec3& point_on_plane,
                                  const Vec3& a,
                                  const Vec3& b,
                                  const Vec3& c) {
    PlaneDistances result{{dot(normal, a - point_on_plane),
                           dot(normal, b - point_on_plane),
                           dot(normal, c - point_on_plane)}};
    for (double& value : result.values) {
        // Möller's implementation used by Open3D applies this epsilon after
        // Open3D's per-axis normalization of the six input vertices.
        if (std::abs(value) < 1e-6) value = 0.0;
    }
    result.all_positive_or_negative =
        (result.values[0] * result.values[1] > 0.0 &&
         result.values[0] * result.values[2] > 0.0);
    return result;
}

std::pair<double, double> plane_triangle_interval(
        const Vec3& a, const Vec3& b, const Vec3& c,
        const std::array<double, 3>& distances,
        int projection_axis) {
    const std::array<Vec3, 3> points{a, b, c};
    std::array<double, 2> values{};
    std::size_t count = 0;
    for (std::size_t i = 0; i < 3 && count < 2; ++i) {
        const std::size_t j = (i + 1) % 3;
        const double di = distances[i];
        const double dj = distances[j];
        if (di == 0.0) {
            values[count++] = component(points[i], projection_axis);
        } else if ((di > 0.0) != (dj > 0.0)) {
            const double ratio = di / (di - dj);
            values[count++] = component(points[i], projection_axis) +
                              ratio * (component(points[j], projection_axis) -
                                       component(points[i], projection_axis));
        }
    }
    if (count == 0) {
        throw std::logic_error("triangle-plane interval is empty");
    }
    if (count == 1) values[1] = values[0];
    if (values[0] > values[1]) std::swap(values[0], values[1]);
    return {values[0], values[1]};
}

bool triangle_triangle_intersects_normalized(
        const Vec3& p0, const Vec3& p1, const Vec3& p2,
        const Vec3& q0, const Vec3& q1, const Vec3& q2) {
    const Vec3 p_normal = cross(p1 - p0, p2 - p0);
    const PlaneDistances q_distances = distances_to_plane(p_normal, p0, q0, q1, q2);
    if (q_distances.all_positive_or_negative) return false;

    const Vec3 q_normal = cross(q1 - q0, q2 - q0);
    const PlaneDistances p_distances = distances_to_plane(q_normal, q0, p0, p1, p2);
    if (p_distances.all_positive_or_negative) return false;

    const Vec3 direction = cross(p_normal, q_normal);
    const double max_component = std::max({std::abs(direction.x),
                                            std::abs(direction.y),
                                            std::abs(direction.z)});
    if (max_component == 0.0) {
        return coplanar_triangles_intersect(p_normal, p0, p1, p2, q0, q1, q2);
    }
    int axis = 0;
    if (std::abs(direction.y) > std::abs(direction.x)) axis = 1;
    if (std::abs(direction.z) > std::abs(component(direction, axis))) axis = 2;
    const auto p_interval = plane_triangle_interval(p0, p1, p2, p_distances.values, axis);
    const auto q_interval = plane_triangle_interval(q0, q1, q2, q_distances.values, axis);
    return !(p_interval.second < q_interval.first || q_interval.second < p_interval.first);
}

// DEPRECATED: This is the former Tomas Möller-style predicate. It is retained
// as an implementation record for the diagnosed near-coplanar false positive,
// but production self-intersection checks no longer call it.
bool triangle_triangle_intersects(
        const Vec3& p0, const Vec3& p1, const Vec3& p2,
        const Vec3& q0, const Vec3& q1, const Vec3& q2) {
    const Vec3 mean = (p0 + p1 + p2 + q0 + q1 + q2) / 6.0;
    const auto sigma_axis = [&](int axis) {
        double squared_sum = 0.0;
        for (const Vec3* point : {&p0, &p1, &p2, &q0, &q1, &q2}) {
            const double delta = component(*point, axis) - component(mean, axis);
            squared_sum += delta * delta;
        }
        return std::sqrt(squared_sum / 5.0) + 1e-12;
    };
    const Vec3 sigma{sigma_axis(0), sigma_axis(1), sigma_axis(2)};
    const auto normalize = [&](const Vec3& point) {
        return Vec3{(point.x - mean.x) / sigma.x,
                    (point.y - mean.y) / sigma.y,
                    (point.z - mean.z) / sigma.z};
    };
    return triangle_triangle_intersects_normalized(
        normalize(p0), normalize(p1), normalize(p2),
        normalize(q0), normalize(q1), normalize(q2));
}

bool shares_vertex(const Triangle& a, const Triangle& b) {
    for (std::size_t a_index : a) {
        for (std::size_t b_index : b) {
            if (a_index == b_index) return true;
        }
    }
    return false;
}

[[maybe_unused, deprecated("use fixed::IsSelfIntersecting")]]
bool is_self_intersecting_deprecated(
        const std::vector<Vec3>& vertices,
        const std::vector<Triangle>& triangles) {
    for (std::size_t first = 0; first < triangles.size(); ++first) {
        const Triangle& p = triangles[first];
        const auto p_bounds = triangle_bounds(
            vertices[p[0]], vertices[p[1]], vertices[p[2]]);
        for (std::size_t second = first + 1; second < triangles.size(); ++second) {
            const Triangle& q = triangles[second];
            if (shares_vertex(p, q)) continue;
            const auto q_bounds = triangle_bounds(
                vertices[q[0]], vertices[q[1]], vertices[q[2]]);
            if (aabb_intersects(p_bounds.first, p_bounds.second,
                                q_bounds.first, q_bounds.second) &&
                triangle_triangle_intersects(
                    vertices[p[0]], vertices[p[1]], vertices[p[2]],
                    vertices[q[0]], vertices[q[1]], vertices[q[2]])) {
                return true;
            }
        }
    }
    return false;
}

}  // namespace

TriangleMeshValidator::TriangleMeshValidator(std::vector<Vec3> vertices,
                                             std::vector<Triangle> triangles)
    : vertices_(std::move(vertices)), triangles_(std::move(triangles)) {
    for (const Triangle& triangle : triangles_) {
        for (std::size_t index : triangle) {
            if (index >= vertices_.size()) {
                throw std::invalid_argument("triangle vertex index is out of range");
            }
            if (index > static_cast<std::size_t>(std::numeric_limits<int>::max())) {
                throw std::invalid_argument("triangle vertex index exceeds Open3D's int range");
            }
        }
    }
}

bool TriangleMeshValidator::IsEdgeManifold(bool allow_boundary_edges) const {
    for (const auto& [edge, adjacent] : edge_to_triangles(triangles_)) {
        (void)edge;
        if ((allow_boundary_edges && (adjacent.empty() || adjacent.size() > 2)) ||
            (!allow_boundary_edges && adjacent.size() != 2)) {
            return false;
        }
    }
    return true;
}

bool TriangleMeshValidator::IsVertexManifold() const {
    std::vector<std::set<std::size_t>> vertex_to_triangles(vertices_.size());
    for (std::size_t triangle_index = 0; triangle_index < triangles_.size(); ++triangle_index) {
        for (std::size_t vertex_index : triangles_[triangle_index]) {
            vertex_to_triangles[vertex_index].insert(triangle_index);
        }
    }
    for (std::size_t vertex_index = 0; vertex_index < vertices_.size(); ++vertex_index) {
        const auto& incident = vertex_to_triangles[vertex_index];
        if (incident.empty()) continue;

        std::map<std::size_t, std::set<std::size_t>> opposite_edges;
        for (std::size_t triangle_index : incident) {
            const Triangle& triangle = triangles_[triangle_index];
            if (triangle[0] != vertex_index && triangle[1] != vertex_index) {
                opposite_edges[triangle[0]].insert(triangle[1]);
                opposite_edges[triangle[1]].insert(triangle[0]);
            } else if (triangle[0] != vertex_index && triangle[2] != vertex_index) {
                opposite_edges[triangle[0]].insert(triangle[2]);
                opposite_edges[triangle[2]].insert(triangle[0]);
            } else if (triangle[1] != vertex_index && triangle[2] != vertex_index) {
                opposite_edges[triangle[1]].insert(triangle[2]);
                opposite_edges[triangle[2]].insert(triangle[1]);
            }
        }
        if (opposite_edges.empty()) {
            throw std::domain_error("vertex manifold is undefined for a fully collapsed triangle");
        }

        std::queue<std::size_t> pending;
        std::set<std::size_t> visited;
        pending.push(opposite_edges.begin()->first);
        visited.insert(opposite_edges.begin()->first);
        while (!pending.empty()) {
            const std::size_t current = pending.front();
            pending.pop();
            for (std::size_t neighbor : opposite_edges[current]) {
                if (visited.insert(neighbor).second) pending.push(neighbor);
            }
        }
        if (visited.size() != opposite_edges.size()) return false;
    }
    return true;
}

bool TriangleMeshValidator::IsSelfIntersecting() const {
    return fixed::IsSelfIntersecting(vertices_, triangles_);
}

bool TriangleMeshValidator::IsOrientable() const {
    std::unordered_map<Edge, Edge, EdgeHash> edge_orientation;
    std::unordered_set<std::size_t, TriangleIndexHash> unvisited;
    std::unordered_map<Edge,
                       std::unordered_set<std::size_t, TriangleIndexHash>,
                       EdgeHash> adjacent;
    std::queue<std::size_t> pending;
    for (std::size_t index = 0; index < triangles_.size(); ++index) {
        unvisited.insert(index);
        const Triangle& triangle = triangles_[index];
        adjacent[ordered_edge(triangle[0], triangle[1])].insert(index);
        adjacent[ordered_edge(triangle[1], triangle[2])].insert(index);
        adjacent[ordered_edge(triangle[2], triangle[0])].insert(index);
    }
    const auto verify_and_add = [&](std::size_t from, std::size_t to) {
        const Edge key = ordered_edge(from, to);
        const auto existing = edge_orientation.find(key);
        if (existing != edge_orientation.end()) return existing->second.first != from;
        edge_orientation[key] = {from, to};
        return true;
    };
    while (!unvisited.empty()) {
        std::size_t triangle_index;
        if (pending.empty()) {
            triangle_index = *unvisited.begin();
        } else {
            triangle_index = pending.front();
            pending.pop();
        }
        if (unvisited.erase(triangle_index) == 0) continue;

        Triangle triangle = triangles_[triangle_index];
        const Edge edge01 = ordered_edge(triangle[0], triangle[1]);
        const Edge edge12 = ordered_edge(triangle[1], triangle[2]);
        const Edge edge20 = ordered_edge(triangle[2], triangle[0]);
        const bool exists01 = edge_orientation.count(edge01) != 0;
        const bool exists12 = edge_orientation.count(edge12) != 0;
        const bool exists20 = edge_orientation.count(edge20) != 0;
        if (!(exists01 || exists12 || exists20)) {
            edge_orientation[edge01] = {triangle[0], triangle[1]};
            edge_orientation[edge12] = {triangle[1], triangle[2]};
            edge_orientation[edge20] = {triangle[2], triangle[0]};
        } else {
            if (exists01 && edge_orientation.at(edge01).first == triangle[0]) {
                std::swap(triangle[0], triangle[1]);
            } else if (exists12 && edge_orientation.at(edge12).first == triangle[1]) {
                std::swap(triangle[1], triangle[2]);
            } else if (exists20 && edge_orientation.at(edge20).first == triangle[2]) {
                std::swap(triangle[2], triangle[0]);
            }
            if (!verify_and_add(triangle[0], triangle[1]) ||
                !verify_and_add(triangle[1], triangle[2]) ||
                !verify_and_add(triangle[2], triangle[0])) {
                return false;
            }
        }
        for (const Edge& edge : {edge01, edge12, edge20}) {
            for (std::size_t neighbor : adjacent[edge]) pending.push(neighbor);
        }
    }
    return true;
}

bool TriangleMeshValidator::IsWatertight() const {
    return IsEdgeManifold(false) && IsVertexManifold() && !IsSelfIntersecting();
}

bool TriangleMeshValidator::HasDegenerateTriangles() const {
    for (const Triangle& triangle : triangles_) {
        if (triangle[0] == triangle[1] || triangle[1] == triangle[2] ||
            triangle[2] == triangle[0]) {
            return true;
        }
    }
    return false;
}

TriangleMeshValidator& TriangleMeshValidator::MergeCloseVertices(double eps) {
    if (!std::isfinite(eps) || eps <= 0.0) {
        throw std::invalid_argument("merge distance must be finite and positive");
    }
    if (vertices_.size() > static_cast<std::size_t>(std::numeric_limits<int>::max())) {
        throw std::invalid_argument("vertex count exceeds Open3D's int range");
    }
    for (const Vec3& vertex : vertices_) {
        if (!std::isfinite(vertex.x) || !std::isfinite(vertex.y) ||
            !std::isfinite(vertex.z)) {
            throw std::invalid_argument("mesh contains a non-finite vertex coordinate");
        }
    }

    std::vector<std::size_t> kd_indices(vertices_.size());
    for (std::size_t index = 0; index < kd_indices.size(); ++index) {
        kd_indices[index] = index;
    }
    std::vector<KdNode> kd_nodes;
    kd_nodes.reserve(vertices_.size());
    const std::size_t kd_root =
        build_kd_tree(kd_indices, 0, kd_indices.size(), 0, vertices_, kd_nodes);

    std::vector<std::size_t> old_to_new(vertices_.size(), kNoIndex);
    std::vector<Vec3> merged_vertices;
    merged_vertices.reserve(vertices_.size());
    std::vector<std::size_t> neighbors;
    const double eps_squared = eps * eps;
    for (std::size_t vertex_index = 0; vertex_index < vertices_.size();
         ++vertex_index) {
        if (old_to_new[vertex_index] != kNoIndex) continue;

        const std::size_t merged_index = merged_vertices.size();
        old_to_new[vertex_index] = merged_index;
        Vec3 merged = vertices_[vertex_index];
        std::size_t merged_count = 1;

        neighbors.clear();
        radius_search(kd_nodes, kd_root, vertices_, vertices_[vertex_index],
                      eps_squared, neighbors);
        std::sort(neighbors.begin(), neighbors.end(), [&](std::size_t left,
                                                          std::size_t right) {
            const Vec3& origin = vertices_[vertex_index];
            const Vec3& left_vertex = vertices_[left];
            const Vec3& right_vertex = vertices_[right];
            const double left_dx = left_vertex.x - origin.x;
            const double left_dy = left_vertex.y - origin.y;
            const double left_dz = left_vertex.z - origin.z;
            const double right_dx = right_vertex.x - origin.x;
            const double right_dy = right_vertex.y - origin.y;
            const double right_dz = right_vertex.z - origin.z;
            const double left_distance =
                left_dx * left_dx + left_dy * left_dy + left_dz * left_dz;
            const double right_distance =
                right_dx * right_dx + right_dy * right_dy + right_dz * right_dz;
            return left_distance != right_distance ? left_distance < right_distance
                                                   : left < right;
        });
        for (std::size_t neighbor : neighbors) {
            if (neighbor == vertex_index || old_to_new[neighbor] != kNoIndex) {
                continue;
            }
            merged = merged + vertices_[neighbor];
            old_to_new[neighbor] = merged_index;
            ++merged_count;
        }
        merged_vertices.push_back(merged / static_cast<double>(merged_count));
    }

    vertices_.swap(merged_vertices);
    for (Triangle& triangle : triangles_) {
        triangle[0] = old_to_new[triangle[0]];
        triangle[1] = old_to_new[triangle[1]];
        triangle[2] = old_to_new[triangle[2]];
    }
    return *this;
}

TriangleMeshValidator& TriangleMeshValidator::RemoveDegenerateTriangles() {
    std::size_t next = 0;
    for (std::size_t index = 0; index < triangles_.size(); ++index) {
        const Triangle& triangle = triangles_[index];
        if (triangle[0] != triangle[1] && triangle[1] != triangle[2] &&
            triangle[2] != triangle[0]) {
            triangles_[next++] = triangle;
        }
    }
    triangles_.resize(next);
    return *this;
}

TriangleMeshValidator& TriangleMeshValidator::RemoveUnreferencedVertices() {
    std::vector<bool> referenced(vertices_.size(), false);
    for (const Triangle& triangle : triangles_) {
        referenced[triangle[0]] = true;
        referenced[triangle[1]] = true;
        referenced[triangle[2]] = true;
    }

    std::vector<std::size_t> old_to_new(vertices_.size(), kNoIndex);
    std::size_t next = 0;
    for (std::size_t index = 0; index < vertices_.size(); ++index) {
        if (!referenced[index]) continue;
        vertices_[next] = vertices_[index];
        old_to_new[index] = next++;
    }
    vertices_.resize(next);

    for (Triangle& triangle : triangles_) {
        triangle[0] = old_to_new[triangle[0]];
        triangle[1] = old_to_new[triangle[1]];
        triangle[2] = old_to_new[triangle[2]];
    }
    return *this;
}

double TriangleMeshValidator::GetVolume() const {
    if (!IsWatertight()) {
        throw std::domain_error("The mesh is not watertight, and the volume cannot be computed.");
    }
    if (!IsOrientable()) {
        throw std::domain_error("The mesh is not orientable, and the volume cannot be computed.");
    }
    double signed_volume = 0.0;
    for (const Triangle& triangle : triangles_) {
        signed_volume += dot(vertices_[triangle[0]],
                             cross(vertices_[triangle[1]], vertices_[triangle[2]])) / 6.0;
    }
    return std::abs(signed_volume);
}

}  // namespace cadgen_ax::mesh
