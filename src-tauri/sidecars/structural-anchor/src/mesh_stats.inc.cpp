using cadastrophe::mesh::TriangleMeshValidator;
using cadastrophe::mesh::Vec3;

struct TriangleFacet {
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
    bool has_degenerate_triangles = false;
    std::size_t validated_triangles = 0;
    std::size_t raw_vertices = 0;
    std::size_t unique_vertices = 0;
    std::array<double, 3> bbox = {0.0, 0.0, 0.0};
    double bbox_volume = 0.0;
    double solid_volume = 0.0;
    bool has_volume = false;
    bool edge_manifold_closed = false;
    bool edge_manifold_with_boundary = false;
    bool vertex_manifold = false;
    bool orientable = false;
    bool self_intersecting = false;
    bool watertight = false;
    bool topology_approximate = false;
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

struct Vec3Less {
    bool operator()(const Vec3& left, const Vec3& right) const {
        if (left.x != right.x) return left.x < right.x;
        if (left.y != right.y) return left.y < right.y;
        return left.z < right.z;
    }
};

TriangleMeshValidator index_triangle_soup(const std::vector<TriangleFacet>& facets) {
    std::vector<Vec3> vertices;
    std::vector<cadastrophe::mesh::Triangle> triangles;
    std::map<Vec3, std::size_t, Vec3Less> vertex_indices;
    vertices.reserve(facets.size() * 3);
    triangles.reserve(facets.size());

    const auto get_index = [&](const Vec3& vertex) -> std::size_t {
        if (!std::isfinite(vertex.x) || !std::isfinite(vertex.y) ||
            !std::isfinite(vertex.z)) {
            throw std::runtime_error("STL contains a non-finite vertex coordinate");
        }
        const auto existing = vertex_indices.find(vertex);
        if (existing != vertex_indices.end()) return existing->second;
        const std::size_t index = vertices.size();
        vertices.push_back(vertex);
        vertex_indices.emplace(vertex, index);
        return index;
    };

    for (const TriangleFacet& facet : facets) {
        triangles.push_back({get_index(facet.a), get_index(facet.b), get_index(facet.c)});
    }
    return TriangleMeshValidator(std::move(vertices), std::move(triangles));
}

void finalize_mesh_stats(MeshStats& stats, const std::vector<TriangleFacet>& facets) {
    TriangleMeshValidator mesh = index_triangle_soup(facets);
    stats.raw_triangles = facets.size();
    stats.validated_triangles = facets.size();
    stats.raw_vertices = facets.size() * 3;
    stats.unique_vertices = mesh.vertices().size();

    Vec3 min_v;
    Vec3 max_v;
    bool has_bbox = false;
    for (const Vec3& vertex : mesh.vertices()) {
        update_bbox(vertex, min_v, max_v, has_bbox);
    }
    if (has_bbox) {
        stats.bbox = {max_v.x - min_v.x, max_v.y - min_v.y, max_v.z - min_v.z};
        stats.bbox_volume = stats.bbox[0] * stats.bbox[1] * stats.bbox[2];
    }

    for (const cadastrophe::mesh::Triangle& triangle : mesh.triangles()) {
        if (triangle[0] == triangle[1] || triangle[1] == triangle[2] ||
            triangle[2] == triangle[0]) {
            ++stats.degenerate_triangles;
        }
    }
    stats.has_degenerate_triangles = mesh.HasDegenerateTriangles();
    stats.edge_manifold_closed = mesh.IsEdgeManifold(false);
    stats.edge_manifold_with_boundary = mesh.IsEdgeManifold(true);
    stats.vertex_manifold = mesh.IsVertexManifold();
    stats.orientable = mesh.IsOrientable();
    stats.self_intersecting = mesh.IsSelfIntersecting();
    stats.watertight = mesh.IsWatertight();
    if (stats.watertight && stats.orientable) {
        stats.solid_volume = mesh.GetVolume();
        stats.has_volume = stats.solid_volume > 0.0;
    }
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

std::vector<TriangleFacet> parse_binary_stl(const std::vector<std::uint8_t>& bytes) {
    const std::uint32_t triangle_count = read_u32_le(bytes, 80);
    std::vector<TriangleFacet> triangles;
    triangles.reserve(triangle_count);
    std::size_t offset = 84;
    for (std::uint32_t i = 0; i < triangle_count; ++i) {
        offset += 12;
        TriangleFacet triangle;
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

std::vector<TriangleFacet> parse_ascii_stl(const std::string& text) {
    std::istringstream in(text);
    std::string line;
    std::vector<Vec3> pending_vertices;
    std::vector<TriangleFacet> triangles;
    while (std::getline(in, line)) {
        std::istringstream words(line);
        std::string token;
        words >> token;
        if (token != "vertex") continue;
        Vec3 vertex;
        if (!(words >> vertex.x >> vertex.y >> vertex.z)) {
            throw std::runtime_error("ASCII STL contains a malformed vertex");
        }
        pending_vertices.push_back(vertex);
        if (pending_vertices.size() == 3) {
            triangles.push_back({pending_vertices[0], pending_vertices[1], pending_vertices[2]});
            pending_vertices.clear();
        }
    }
    if (!pending_vertices.empty()) {
        throw std::runtime_error("ASCII STL contains an incomplete triangle facet");
    }
    return triangles;
}

MeshStats load_mesh(const fs::path& path) {
    MeshStats stats;
    const std::vector<std::uint8_t> bytes = read_binary_file(path);
    stats.bytes = bytes.size();
    stats.sha256 = sha256_hex(bytes);
    std::vector<TriangleFacet> triangles;
    if (bytes.size() >= 84) {
        const std::uint32_t triangle_count = read_u32_le(bytes, 80);
        const std::uint64_t expected_size = 84ULL + static_cast<std::uint64_t>(triangle_count) * 50ULL;
        if (expected_size == bytes.size()) triangles = parse_binary_stl(bytes);
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
