struct Vec3 {
    double x = 0.0;
    double y = 0.0;
    double z = 0.0;
};

Vec3 operator-(const Vec3& a, const Vec3& b) {
    return {a.x - b.x, a.y - b.y, a.z - b.z};
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

double norm(const Vec3& v) {
    return std::sqrt(dot(v, v));
}

std::string vertex_key(const Vec3& v) {
    std::ostringstream out;
    out << std::fixed << std::setprecision(6) << v.x << "," << v.y << "," << v.z;
    return out.str();
}

struct Triangle {
    Vec3 a;
    Vec3 b;
    Vec3 c;
};

struct MeshStats {
    bool loaded = false;
    std::string engine = "native-stl-parser";
    std::string error;
    std::uint64_t bytes = 0;
    std::string sha256;
    std::size_t raw_triangles = 0;
    std::size_t degenerate_triangles = 0;
    std::size_t triangles_after_cleanup = 0;
    std::size_t raw_vertices = 0;
    std::size_t unique_vertices = 0;
    std::array<double, 3> bbox = {0.0, 0.0, 0.0};
    double bbox_volume = 0.0;
    double solid_volume = 0.0;
    bool edge_manifold_closed = false;
    bool edge_manifold_with_boundary = false;
    bool vertex_manifold = false;
    bool orientable = false;
    bool self_intersecting = false;
    bool topology_approximate = true;
};

void update_bbox(const Vec3& v, Vec3& min_v, Vec3& max_v, bool& initialized) {
    if (!initialized) {
        min_v = v;
        max_v = v;
        initialized = true;
        return;
    }
    min_v.x = std::min(min_v.x, v.x);
    min_v.y = std::min(min_v.y, v.y);
    min_v.z = std::min(min_v.z, v.z);
    max_v.x = std::max(max_v.x, v.x);
    max_v.y = std::max(max_v.y, v.y);
    max_v.z = std::max(max_v.z, v.z);
}

bool is_degenerate(const Triangle& triangle) {
    return norm(cross(triangle.b - triangle.a, triangle.c - triangle.a)) <= 1e-9;
}

void finalize_mesh_stats(MeshStats& stats, const std::vector<Triangle>& triangles) {
    stats.raw_triangles = triangles.size();
    stats.raw_vertices = triangles.size() * 3;

    std::set<std::string> vertices;
    std::map<std::pair<std::string, std::string>, int> edge_counts;
    Vec3 min_v;
    Vec3 max_v;
    bool has_bbox = false;
    double signed_volume = 0.0;

    for (const Triangle& triangle : triangles) {
        update_bbox(triangle.a, min_v, max_v, has_bbox);
        update_bbox(triangle.b, min_v, max_v, has_bbox);
        update_bbox(triangle.c, min_v, max_v, has_bbox);
        const bool degenerate = is_degenerate(triangle);
        if (degenerate) {
            ++stats.degenerate_triangles;
            continue;
        }
        const std::array<std::string, 3> keys = {
            vertex_key(triangle.a),
            vertex_key(triangle.b),
            vertex_key(triangle.c),
        };
        vertices.insert(keys[0]);
        vertices.insert(keys[1]);
        vertices.insert(keys[2]);
        for (const auto& edge : {std::pair<std::string, std::string>{keys[0], keys[1]},
                                 std::pair<std::string, std::string>{keys[1], keys[2]},
                                 std::pair<std::string, std::string>{keys[2], keys[0]}}) {
            auto sorted = edge.first < edge.second ? edge : std::pair<std::string, std::string>{edge.second, edge.first};
            edge_counts[sorted] += 1;
        }
        signed_volume += dot(triangle.a, cross(triangle.b, triangle.c)) / 6.0;
    }

    stats.triangles_after_cleanup = stats.raw_triangles - stats.degenerate_triangles;
    stats.unique_vertices = vertices.size();
    if (has_bbox) {
        stats.bbox = {max_v.x - min_v.x, max_v.y - min_v.y, max_v.z - min_v.z};
        stats.bbox_volume = stats.bbox[0] * stats.bbox[1] * stats.bbox[2];
    }
    stats.solid_volume = std::fabs(signed_volume) > 1e-9 ? std::fabs(signed_volume) : stats.bbox_volume;

    bool all_edges_closed = !edge_counts.empty();
    bool all_edges_have_boundary_or_closed = !edge_counts.empty();
    for (const auto& [edge, count] : edge_counts) {
        (void)edge;
        all_edges_closed = all_edges_closed && count == 2;
        all_edges_have_boundary_or_closed = all_edges_have_boundary_or_closed && (count == 1 || count == 2);
    }
    stats.edge_manifold_closed = all_edges_closed;
    stats.edge_manifold_with_boundary = all_edges_have_boundary_or_closed;
    stats.vertex_manifold = stats.unique_vertices > 0 && stats.triangles_after_cleanup > 0 && all_edges_have_boundary_or_closed;
    stats.orientable = all_edges_closed;
    stats.self_intersecting = false;
    stats.loaded = true;
}

float read_float_le(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    std::uint32_t raw = static_cast<std::uint32_t>(bytes[offset])
        | (static_cast<std::uint32_t>(bytes[offset + 1]) << 8)
        | (static_cast<std::uint32_t>(bytes[offset + 2]) << 16)
        | (static_cast<std::uint32_t>(bytes[offset + 3]) << 24);
    float value = 0.0f;
    std::memcpy(&value, &raw, sizeof(float));
    return value;
}

std::uint32_t read_u32_le(const std::vector<std::uint8_t>& bytes, std::size_t offset) {
    return static_cast<std::uint32_t>(bytes[offset])
        | (static_cast<std::uint32_t>(bytes[offset + 1]) << 8)
        | (static_cast<std::uint32_t>(bytes[offset + 2]) << 16)
        | (static_cast<std::uint32_t>(bytes[offset + 3]) << 24);
}

std::vector<Triangle> parse_binary_stl(const std::vector<std::uint8_t>& bytes) {
    const std::uint32_t triangle_count = read_u32_le(bytes, 80);
    std::vector<Triangle> triangles;
    triangles.reserve(triangle_count);
    std::size_t offset = 84;
    for (std::uint32_t i = 0; i < triangle_count; ++i) {
        offset += 12;
        Triangle triangle;
        triangle.a = {read_float_le(bytes, offset), read_float_le(bytes, offset + 4), read_float_le(bytes, offset + 8)};
        offset += 12;
        triangle.b = {read_float_le(bytes, offset), read_float_le(bytes, offset + 4), read_float_le(bytes, offset + 8)};
        offset += 12;
        triangle.c = {read_float_le(bytes, offset), read_float_le(bytes, offset + 4), read_float_le(bytes, offset + 8)};
        offset += 14;
        triangles.push_back(triangle);
    }
    return triangles;
}

std::vector<Triangle> parse_ascii_stl(const std::string& text) {
    std::istringstream in(text);
    std::string line;
    std::vector<Vec3> pending_vertices;
    std::vector<Triangle> triangles;
    while (std::getline(in, line)) {
        std::istringstream words(line);
        std::string token;
        words >> token;
        if (token != "vertex") {
            continue;
        }
        Vec3 vertex;
        if (words >> vertex.x >> vertex.y >> vertex.z) {
            pending_vertices.push_back(vertex);
            if (pending_vertices.size() == 3) {
                triangles.push_back({pending_vertices[0], pending_vertices[1], pending_vertices[2]});
                pending_vertices.clear();
            }
        }
    }
    return triangles;
}

MeshStats load_mesh(const fs::path& path) {
    MeshStats stats;
    const std::vector<std::uint8_t> bytes = read_binary_file(path);
    stats.bytes = bytes.size();
    stats.sha256 = sha256_hex(bytes);
    std::vector<Triangle> triangles;
    if (bytes.size() >= 84) {
        const std::uint32_t triangle_count = read_u32_le(bytes, 80);
        const std::uint64_t expected_size = 84ULL + static_cast<std::uint64_t>(triangle_count) * 50ULL;
        if (expected_size == bytes.size()) {
            triangles = parse_binary_stl(bytes);
        }
    }
    if (triangles.empty()) {
        const std::string text(bytes.begin(), bytes.end());
        triangles = parse_ascii_stl(text);
    }
    if (triangles.empty()) {
        stats.error = "STL parser found no triangle facets.";
        return stats;
    }
    finalize_mesh_stats(stats, triangles);
    return stats;
}
