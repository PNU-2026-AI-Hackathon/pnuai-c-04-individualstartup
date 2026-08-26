#include "mesh_intersection_fixed.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <limits>
#include <stdexcept>
#include <utility>
#include <vector>

namespace cadgen_ax::mesh::fixed {
namespace {

enum class SliceKind {
    Empty,
    Point,
    Segment,
    Coplanar,
};

struct PlaneSlice {
    SliceKind kind = SliceKind::Empty;
    std::array<Vec3, 2> points{};
};

struct UnitPlane {
    Vec3 origin;
    Vec3 normal;
};

Vec3 add(const Vec3& a, const Vec3& b) {
    return {a.x + b.x, a.y + b.y, a.z + b.z};
}

Vec3 subtract(const Vec3& a, const Vec3& b) {
    return {a.x - b.x, a.y - b.y, a.z - b.z};
}

Vec3 multiply(const Vec3& value, double factor) {
    return {value.x * factor, value.y * factor, value.z * factor};
}

double dot(const Vec3& a, const Vec3& b) {
    return a.x * b.x + a.y * b.y + a.z * b.z;
}

Vec3 cross(const Vec3& a, const Vec3& b) {
    return {a.y * b.z - a.z * b.y,
            a.z * b.x - a.x * b.z,
            a.x * b.y - a.y * b.x};
}

double length(const Vec3& value) {
    return std::sqrt(dot(value, value));
}

double component(const Vec3& value, int axis) {
    if (axis == 0) return value.x;
    if (axis == 1) return value.y;
    return value.z;
}

bool finite(const Vec3& value) {
    return std::isfinite(value.x) && std::isfinite(value.y) &&
           std::isfinite(value.z);
}

void validate_tolerance(const IntersectionTolerance& tolerance) {
    if (!std::isfinite(tolerance.plane_distance) ||
        tolerance.plane_distance <= 0.0) {
        throw std::invalid_argument(
            "plane_distance tolerance must be finite and greater than zero");
    }
    if (!std::isfinite(tolerance.normal_sine) ||
        tolerance.normal_sine <= 0.0 || tolerance.normal_sine >= 1.0) {
        throw std::invalid_argument(
            "normal_sine tolerance must be finite and in the open interval (0, 1)");
    }
}

UnitPlane make_unit_plane(const Vec3& a, const Vec3& b, const Vec3& c) {
    const Vec3 raw_normal = cross(subtract(b, a), subtract(c, a));
    const double normal_length = length(raw_normal);
    if (!std::isfinite(normal_length) || normal_length == 0.0) {
        throw std::domain_error(
            "triangle/triangle intersection is undefined for a zero-area triangle");
    }
    return {a, multiply(raw_normal, 1.0 / normal_length)};
}

std::array<double, 3> signed_plane_values(const UnitPlane& plane,
                                          const Vec3& a,
                                          const Vec3& b,
                                          const Vec3& c) {
    return {dot(plane.normal, subtract(a, plane.origin)),
            dot(plane.normal, subtract(b, plane.origin)),
            dot(plane.normal, subtract(c, plane.origin))};
}

bool all_greater_than(const std::array<double, 3>& values, double threshold) {
    return std::all_of(values.begin(), values.end(),
                       [&](double value) { return value > threshold; });
}

bool all_less_than(const std::array<double, 3>& values, double threshold) {
    return std::all_of(values.begin(), values.end(),
                       [&](double value) { return value < threshold; });
}

bool all_near_zero(const std::array<double, 3>& values, double epsilon) {
    return std::all_of(values.begin(), values.end(),
                       [&](double value) { return std::abs(value) <= epsilon; });
}

void add_unique_point(std::vector<Vec3>& points,
                      const Vec3& candidate,
                      double epsilon) {
    for (const Vec3& point : points) {
        if (length(subtract(candidate, point)) <= epsilon) return;
    }
    points.push_back(candidate);
}

PlaneSlice slice_triangle(const Vec3& a,
                          const Vec3& b,
                          const Vec3& c,
                          const std::array<double, 3>& distances,
                          double epsilon) {
    if (all_near_zero(distances, epsilon)) {
        return {SliceKind::Coplanar, {}};
    }
    if (all_greater_than(distances, epsilon) ||
        all_less_than(distances, -epsilon)) {
        return {SliceKind::Empty, {}};
    }

    const std::array<Vec3, 3> vertices{a, b, c};
    std::vector<Vec3> points;
    points.reserve(2);

    // Inspect all vertices first. Unlike the legacy routine, this never stops
    // merely because the first two input vertices happened to be near a plane.
    for (std::size_t i = 0; i < 3; ++i) {
        if (std::abs(distances[i]) <= epsilon) {
            add_unique_point(points, vertices[i], epsilon);
        }
    }

    // Only edges whose endpoints are unambiguously on opposite sides create an
    // interpolated crossing. An endpoint in the epsilon band was added above.
    for (std::size_t i = 0; i < 3; ++i) {
        const std::size_t j = (i + 1) % 3;
        const double di = distances[i];
        const double dj = distances[j];
        if ((di < -epsilon && dj > epsilon) ||
            (di > epsilon && dj < -epsilon)) {
            const double ratio = di / (di - dj);
            add_unique_point(
                points,
                add(vertices[i], multiply(subtract(vertices[j], vertices[i]), ratio)),
                epsilon);
        }
    }

    if (points.empty()) {
        throw std::logic_error(
            "non-coplanar triangle/plane slice produced no intersection point");
    }
    if (points.size() > 2) {
        throw std::logic_error(
            "non-coplanar triangle/plane slice produced more than two points");
    }
    if (points.size() == 1) {
        return {SliceKind::Point, {points[0], points[0]}};
    }
    return {SliceKind::Segment, {points[0], points[1]}};
}

std::pair<double, double> projected_interval(const PlaneSlice& slice, int axis) {
    if (slice.kind != SliceKind::Point && slice.kind != SliceKind::Segment) {
        throw std::logic_error(
            "only a point or segment can be projected to an intersection interval");
    }
    double first = component(slice.points[0], axis);
    double second = component(slice.points[1], axis);
    if (first > second) std::swap(first, second);
    return {first, second};
}

std::array<double, 2> project_2d(const Vec3& point, int drop_axis) {
    if (drop_axis == 0) return {point.y, point.z};
    if (drop_axis == 1) return {point.x, point.z};
    return {point.x, point.y};
}

bool separated_on_axis(const std::array<std::array<double, 2>, 3>& p,
                       const std::array<std::array<double, 2>, 3>& q,
                       const std::array<double, 2>& raw_axis,
                       double epsilon) {
    const double axis_length = std::hypot(raw_axis[0], raw_axis[1]);
    if (axis_length == 0.0) {
        throw std::domain_error(
            "coplanar triangle projection contains a zero-length edge");
    }
    const std::array<double, 2> axis{raw_axis[0] / axis_length,
                                     raw_axis[1] / axis_length};
    const auto projection = [&](const std::array<double, 2>& point) {
        return point[0] * axis[0] + point[1] * axis[1];
    };

    double p_min = projection(p[0]);
    double p_max = p_min;
    double q_min = projection(q[0]);
    double q_max = q_min;
    for (std::size_t i = 1; i < 3; ++i) {
        p_min = std::min(p_min, projection(p[i]));
        p_max = std::max(p_max, projection(p[i]));
        q_min = std::min(q_min, projection(q[i]));
        q_max = std::max(q_max, projection(q[i]));
    }
    return p_max < q_min - epsilon || q_max < p_min - epsilon;
}

bool coplanar_triangles_intersect(const Vec3& unit_normal,
                                  const Vec3& p0,
                                  const Vec3& p1,
                                  const Vec3& p2,
                                  const Vec3& q0,
                                  const Vec3& q1,
                                  const Vec3& q2,
                                  double epsilon) {
    int drop_axis = 0;
    if (std::abs(unit_normal.y) > std::abs(unit_normal.x)) drop_axis = 1;
    if (std::abs(unit_normal.z) > std::abs(component(unit_normal, drop_axis))) {
        drop_axis = 2;
    }

    const std::array<std::array<double, 2>, 3> p{
        project_2d(p0, drop_axis), project_2d(p1, drop_axis),
        project_2d(p2, drop_axis)};
    const std::array<std::array<double, 2>, 3> q{
        project_2d(q0, drop_axis), project_2d(q1, drop_axis),
        project_2d(q2, drop_axis)};

    // Separating Axis Theorem for two closed convex triangles. Axes are the
    // in-plane normals of all six edges.
    for (const auto* triangle : {&p, &q}) {
        for (std::size_t i = 0; i < 3; ++i) {
            const auto& a = (*triangle)[i];
            const auto& b = (*triangle)[(i + 1) % 3];
            const std::array<double, 2> edge{b[0] - a[0], b[1] - a[1]};
            const std::array<double, 2> axis{-edge[1], edge[0]};
            if (separated_on_axis(p, q, axis, epsilon)) return false;
        }
    }
    return true;
}

bool shares_vertex(const Triangle& p, const Triangle& q) {
    for (std::size_t p_index : p) {
        for (std::size_t q_index : q) {
            if (p_index == q_index) return true;
        }
    }
    return false;
}

std::pair<Vec3, Vec3> bounds(const Vec3& a, const Vec3& b, const Vec3& c) {
    return {{std::min({a.x, b.x, c.x}), std::min({a.y, b.y, c.y}),
             std::min({a.z, b.z, c.z})},
            {std::max({a.x, b.x, c.x}), std::max({a.y, b.y, c.y}),
             std::max({a.z, b.z, c.z})}};
}

bool aabb_intersects(const std::pair<Vec3, Vec3>& p,
                     const std::pair<Vec3, Vec3>& q) {
    return !(p.second.x < q.first.x || p.first.x > q.second.x ||
             p.second.y < q.first.y || p.first.y > q.second.y ||
             p.second.z < q.first.z || p.first.z > q.second.z);
}

void validate_mesh_input(const std::vector<Vec3>& vertices,
                         const std::vector<Triangle>& triangles,
                         const IntersectionTolerance& tolerance) {
    validate_tolerance(tolerance);
    for (const Vec3& vertex : vertices) {
        if (!finite(vertex)) {
            throw std::invalid_argument(
                "self-intersection requires finite vertex coordinates");
        }
    }
    for (const Triangle& triangle : triangles) {
        for (std::size_t index : triangle) {
            if (index >= vertices.size()) {
                throw std::invalid_argument("triangle vertex index is out of range");
            }
        }
    }
}

}  // namespace

bool TriangleTriangleIntersects(const Vec3& p0,
                                const Vec3& p1,
                                const Vec3& p2,
                                const Vec3& q0,
                                const Vec3& q1,
                                const Vec3& q2,
                                const IntersectionTolerance& tolerance) {
    validate_tolerance(tolerance);
    for (const Vec3* point : {&p0, &p1, &p2, &q0, &q1, &q2}) {
        if (!finite(*point)) {
            throw std::invalid_argument(
                "triangle/triangle intersection requires finite coordinates");
        }
    }

    const Vec3 mean{(p0.x + p1.x + p2.x + q0.x + q1.x + q2.x) / 6.0,
                    (p0.y + p1.y + p2.y + q0.y + q1.y + q2.y) / 6.0,
                    (p0.z + p1.z + p2.z + q0.z + q1.z + q2.z) / 6.0};
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

    const Vec3 np0 = normalize(p0);
    const Vec3 np1 = normalize(p1);
    const Vec3 np2 = normalize(p2);
    const Vec3 nq0 = normalize(q0);
    const Vec3 nq1 = normalize(q1);
    const Vec3 nq2 = normalize(q2);
    const UnitPlane p_plane = make_unit_plane(np0, np1, np2);
    const UnitPlane q_plane = make_unit_plane(nq0, nq1, nq2);
    const auto q_to_p = signed_plane_values(p_plane, nq0, nq1, nq2);
    const auto p_to_q = signed_plane_values(q_plane, np0, np1, np2);

    if (all_greater_than(q_to_p, tolerance.plane_distance) ||
        all_less_than(q_to_p, -tolerance.plane_distance) ||
        all_greater_than(p_to_q, tolerance.plane_distance) ||
        all_less_than(p_to_q, -tolerance.plane_distance)) {
        return false;
    }

    const double normal_sine = length(cross(p_plane.normal, q_plane.normal));
    const bool parallel = normal_sine <= tolerance.normal_sine;
    const PlaneSlice p_slice = slice_triangle(
        np0, np1, np2, p_to_q, tolerance.plane_distance);
    const PlaneSlice q_slice = slice_triangle(
        nq0, nq1, nq2, q_to_p, tolerance.plane_distance);

    if (parallel) {
        const bool p_coplanar = p_slice.kind == SliceKind::Coplanar;
        const bool q_coplanar = q_slice.kind == SliceKind::Coplanar;
        if (p_coplanar != q_coplanar) {
            throw std::logic_error(
                "parallel triangle planes produced asymmetric coplanar classification");
        }
        if (!p_coplanar) return false;
        return coplanar_triangles_intersect(
            p_plane.normal, np0, np1, np2, nq0, nq1, nq2,
            tolerance.plane_distance);
    }

    if (p_slice.kind == SliceKind::Coplanar ||
        q_slice.kind == SliceKind::Coplanar) {
        throw std::logic_error(
            "non-parallel triangle planes produced an all-zero plane slice");
    }
    if (p_slice.kind == SliceKind::Empty || q_slice.kind == SliceKind::Empty) {
        return false;
    }

    const Vec3 direction = cross(p_plane.normal, q_plane.normal);
    int axis = 0;
    if (std::abs(direction.y) > std::abs(direction.x)) axis = 1;
    if (std::abs(direction.z) > std::abs(component(direction, axis))) axis = 2;
    const auto p_interval = projected_interval(p_slice, axis);
    const auto q_interval = projected_interval(q_slice, axis);
    return !(p_interval.second < q_interval.first - tolerance.plane_distance ||
             q_interval.second < p_interval.first - tolerance.plane_distance);
}

std::vector<TrianglePair> GetSelfIntersectingTriangles(
        const std::vector<Vec3>& vertices,
        const std::vector<Triangle>& triangles,
        const IntersectionTolerance& tolerance) {
    validate_mesh_input(vertices, triangles, tolerance);

    std::vector<TrianglePair> result;
    for (std::size_t first = 0; first < triangles.size(); ++first) {
        const Triangle& p = triangles[first];
        const auto p_bounds = bounds(vertices[p[0]], vertices[p[1]], vertices[p[2]]);
        for (std::size_t second = first + 1; second < triangles.size(); ++second) {
            const Triangle& q = triangles[second];
            if (shares_vertex(p, q)) continue;
            const auto q_bounds = bounds(vertices[q[0]], vertices[q[1]], vertices[q[2]]);
            if (!aabb_intersects(p_bounds, q_bounds)) continue;
            if (TriangleTriangleIntersects(
                    vertices[p[0]], vertices[p[1]], vertices[p[2]],
                    vertices[q[0]], vertices[q[1]], vertices[q[2]], tolerance)) {
                result.push_back({first, second});
            }
        }
    }
    return result;
}

bool IsSelfIntersecting(const std::vector<Vec3>& vertices,
                        const std::vector<Triangle>& triangles,
                        const IntersectionTolerance& tolerance) {
    validate_mesh_input(vertices, triangles, tolerance);
    for (std::size_t first = 0; first < triangles.size(); ++first) {
        const Triangle& p = triangles[first];
        const auto p_bounds = bounds(vertices[p[0]], vertices[p[1]], vertices[p[2]]);
        for (std::size_t second = first + 1; second < triangles.size(); ++second) {
            const Triangle& q = triangles[second];
            if (shares_vertex(p, q)) continue;
            const auto q_bounds = bounds(vertices[q[0]], vertices[q[1]], vertices[q[2]]);
            if (!aabb_intersects(p_bounds, q_bounds)) continue;
            if (TriangleTriangleIntersects(
                    vertices[p[0]], vertices[p[1]], vertices[p[2]],
                    vertices[q[0]], vertices[q[1]], vertices[q[2]], tolerance)) {
                return true;
            }
        }
    }
    return false;
}

}  // namespace cadgen_ax::mesh::fixed
