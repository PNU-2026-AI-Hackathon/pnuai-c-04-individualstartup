struct Vec3 {
    double x = 0.0;
    double y = 0.0;
    double z = 0.0;
};

Vec3 operator+(const Vec3& a, const Vec3& b) { return {a.x + b.x, a.y + b.y, a.z + b.z}; }
Vec3 operator-(const Vec3& a, const Vec3& b) { return {a.x - b.x, a.y - b.y, a.z - b.z}; }
Vec3 operator*(const Vec3& a, double s) { return {a.x * s, a.y * s, a.z * s}; }

double dot(const Vec3& a, const Vec3& b) { return a.x * b.x + a.y * b.y + a.z * b.z; }
Vec3 cross(const Vec3& a, const Vec3& b) {
    return {a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x};
}
double norm(const Vec3& v) { return std::sqrt(dot(v, v)); }
Vec3 normalized(Vec3 v) {
    const double n = norm(v);
    return n <= 1e-12 ? Vec3{0, 0, 1} : v * (1.0 / n);
}

struct Triangle {
    Vec3 a;
    Vec3 b;
    Vec3 c;
};

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
    std::vector<Vec3> vertices;
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
            vertices.push_back(vertex);
            if (vertices.size() == 3) {
                triangles.push_back({vertices[0], vertices[1], vertices[2]});
                vertices.clear();
            }
        }
    }
    return triangles;
}

std::vector<Triangle> load_stl(const fs::path& path) {
    const std::vector<std::uint8_t> bytes = read_binary_file(path);
    std::vector<Triangle> triangles;
    if (bytes.size() >= 84) {
        const std::uint32_t triangle_count = read_u32_le(bytes, 80);
        const std::uint64_t expected_size = 84ULL + static_cast<std::uint64_t>(triangle_count) * 50ULL;
        if (expected_size == bytes.size()) {
            triangles = parse_binary_stl(bytes);
        }
    }
    if (triangles.empty()) {
        triangles = parse_ascii_stl(std::string(bytes.begin(), bytes.end()));
    }
    if (triangles.empty()) {
        throw std::runtime_error("STL parser found no triangle facets.");
    }
    return triangles;
}

std::vector<Triangle> load_mesh_triangles(const fs::path& path) {
    return load_stl(path);
}
