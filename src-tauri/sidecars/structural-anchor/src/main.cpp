#include <algorithm>
#include <array>
#include <cctype>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <map>
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <utility>
#include <vector>

#if CADASTROPHE_WITH_OPEN3D
#include <open3d/Open3D.h>
#endif

namespace fs = std::filesystem;

namespace {

struct Json {
    enum class Type { Null, Bool, Number, String, Array, Object };

    Type type = Type::Null;
    bool bool_value = false;
    double number_value = 0.0;
    std::string string_value;
    std::vector<Json> array_value;
    std::map<std::string, Json> object_value;

    static Json null() { return Json{}; }
    static Json boolean(bool value) {
        Json json;
        json.type = Type::Bool;
        json.bool_value = value;
        return json;
    }
    static Json number(double value) {
        Json json;
        json.type = Type::Number;
        json.number_value = value;
        return json;
    }
    static Json string(std::string value) {
        Json json;
        json.type = Type::String;
        json.string_value = std::move(value);
        return json;
    }
    static Json array(std::vector<Json> value) {
        Json json;
        json.type = Type::Array;
        json.array_value = std::move(value);
        return json;
    }
    static Json object(std::map<std::string, Json> value) {
        Json json;
        json.type = Type::Object;
        json.object_value = std::move(value);
        return json;
    }

    bool is_object() const { return type == Type::Object; }
    bool is_array() const { return type == Type::Array; }
    bool is_string() const { return type == Type::String; }
    bool is_number() const { return type == Type::Number; }
    bool is_bool() const { return type == Type::Bool; }
    bool is_null() const { return type == Type::Null; }

    const Json* get(const std::string& key) const {
        if (!is_object()) {
            return nullptr;
        }
        auto found = object_value.find(key);
        return found == object_value.end() ? nullptr : &found->second;
    }
};

class JsonParser {
  public:
    explicit JsonParser(std::string input) : input_(std::move(input)) {}

    Json parse() {
        skip_ws();
        Json value = parse_value();
        skip_ws();
        if (pos_ != input_.size()) {
            throw std::runtime_error("unexpected trailing JSON content");
        }
        return value;
    }

  private:
    Json parse_value() {
        skip_ws();
        if (pos_ >= input_.size()) {
            throw std::runtime_error("unexpected end of JSON");
        }
        const char ch = input_[pos_];
        if (ch == '"') {
            return Json::string(parse_string());
        }
        if (ch == '{') {
            return parse_object();
        }
        if (ch == '[') {
            return parse_array();
        }
        if (ch == 't') {
            expect("true");
            return Json::boolean(true);
        }
        if (ch == 'f') {
            expect("false");
            return Json::boolean(false);
        }
        if (ch == 'n') {
            expect("null");
            return Json::null();
        }
        if (ch == '-' || std::isdigit(static_cast<unsigned char>(ch))) {
            return Json::number(parse_number());
        }
        throw std::runtime_error("unexpected JSON token");
    }

    Json parse_object() {
        consume('{');
        std::map<std::string, Json> object;
        skip_ws();
        if (peek('}')) {
            consume('}');
            return Json::object(std::move(object));
        }
        while (true) {
            skip_ws();
            std::string key = parse_string();
            skip_ws();
            consume(':');
            object.emplace(std::move(key), parse_value());
            skip_ws();
            if (peek('}')) {
                consume('}');
                break;
            }
            consume(',');
        }
        return Json::object(std::move(object));
    }

    Json parse_array() {
        consume('[');
        std::vector<Json> array;
        skip_ws();
        if (peek(']')) {
            consume(']');
            return Json::array(std::move(array));
        }
        while (true) {
            array.push_back(parse_value());
            skip_ws();
            if (peek(']')) {
                consume(']');
                break;
            }
            consume(',');
        }
        return Json::array(std::move(array));
    }

    std::string parse_string() {
        consume('"');
        std::string out;
        while (pos_ < input_.size()) {
            char ch = input_[pos_++];
            if (ch == '"') {
                return out;
            }
            if (ch != '\\') {
                out.push_back(ch);
                continue;
            }
            if (pos_ >= input_.size()) {
                throw std::runtime_error("unterminated JSON escape");
            }
            char escaped = input_[pos_++];
            switch (escaped) {
                case '"':
                case '\\':
                case '/':
                    out.push_back(escaped);
                    break;
                case 'b':
                    out.push_back('\b');
                    break;
                case 'f':
                    out.push_back('\f');
                    break;
                case 'n':
                    out.push_back('\n');
                    break;
                case 'r':
                    out.push_back('\r');
                    break;
                case 't':
                    out.push_back('\t');
                    break;
                case 'u':
                    out.push_back('?');
                    if (pos_ + 4 > input_.size()) {
                        throw std::runtime_error("short JSON unicode escape");
                    }
                    pos_ += 4;
                    break;
                default:
                    throw std::runtime_error("invalid JSON escape");
            }
        }
        throw std::runtime_error("unterminated JSON string");
    }

    double parse_number() {
        const std::size_t start = pos_;
        if (peek('-')) {
            ++pos_;
        }
        while (pos_ < input_.size() && std::isdigit(static_cast<unsigned char>(input_[pos_]))) {
            ++pos_;
        }
        if (peek('.')) {
            ++pos_;
            while (pos_ < input_.size() && std::isdigit(static_cast<unsigned char>(input_[pos_]))) {
                ++pos_;
            }
        }
        if (peek('e') || peek('E')) {
            ++pos_;
            if (peek('+') || peek('-')) {
                ++pos_;
            }
            while (pos_ < input_.size() && std::isdigit(static_cast<unsigned char>(input_[pos_]))) {
                ++pos_;
            }
        }
        return std::stod(input_.substr(start, pos_ - start));
    }

    void expect(const char* literal) {
        const std::size_t len = std::strlen(literal);
        if (input_.compare(pos_, len, literal) != 0) {
            throw std::runtime_error("unexpected JSON literal");
        }
        pos_ += len;
    }

    bool peek(char ch) const {
        return pos_ < input_.size() && input_[pos_] == ch;
    }

    void consume(char ch) {
        if (!peek(ch)) {
            throw std::runtime_error(std::string("expected JSON character ") + ch);
        }
        ++pos_;
    }

    void skip_ws() {
        while (pos_ < input_.size() && std::isspace(static_cast<unsigned char>(input_[pos_]))) {
            ++pos_;
        }
    }

    std::string input_;
    std::size_t pos_ = 0;
};

std::string read_text_file(const fs::path& path) {
    std::ifstream in(path, std::ios::binary);
    if (!in) {
        throw std::runtime_error("failed to read file: " + path.string());
    }
    std::ostringstream buffer;
    buffer << in.rdbuf();
    return buffer.str();
}

std::vector<std::uint8_t> read_binary_file(const fs::path& path) {
    std::ifstream in(path, std::ios::binary);
    if (!in) {
        throw std::runtime_error("failed to read file: " + path.string());
    }
    return std::vector<std::uint8_t>(std::istreambuf_iterator<char>(in), {});
}

fs::path resolve_path(const fs::path& base_dir, const std::string& raw_path) {
    fs::path path(raw_path);
    if (path.is_absolute()) {
        return path;
    }
    return fs::weakly_canonical(base_dir / path);
}

std::string get_string(const Json& json, const std::string& key, const std::string& fallback = "") {
    const Json* value = json.get(key);
    return value && value->is_string() ? value->string_value : fallback;
}

double get_number(const Json& json, const std::string& key, double fallback = 0.0) {
    const Json* value = json.get(key);
    return value && value->is_number() ? value->number_value : fallback;
}

bool get_bool(const Json& json, const std::string& key, bool fallback = false) {
    const Json* value = json.get(key);
    return value && value->is_bool() ? value->bool_value : fallback;
}

Json load_json_field_or_path(const Json& input, const fs::path& base_dir, const std::string& field, const std::string& path_field) {
    if (const Json* inline_json = input.get(field)) {
        return *inline_json;
    }
    const std::string path = get_string(input, path_field);
    if (!path.empty()) {
        return JsonParser(read_text_file(resolve_path(base_dir, path))).parse();
    }
    return Json::object({});
}

std::string escape_json_string(const std::string& input) {
    std::ostringstream out;
    for (unsigned char ch : input) {
        switch (ch) {
            case '"':
                out << "\\\"";
                break;
            case '\\':
                out << "\\\\";
                break;
            case '\b':
                out << "\\b";
                break;
            case '\f':
                out << "\\f";
                break;
            case '\n':
                out << "\\n";
                break;
            case '\r':
                out << "\\r";
                break;
            case '\t':
                out << "\\t";
                break;
            default:
                if (ch < 0x20) {
                    out << "\\u" << std::hex << std::setw(4) << std::setfill('0') << static_cast<int>(ch);
                } else {
                    out << ch;
                }
        }
    }
    return out.str();
}

std::string format_number(double value) {
    if (!std::isfinite(value)) {
        return "null";
    }
    if (std::fabs(value) < 0.0000000005) {
        value = 0.0;
    }
    std::ostringstream out;
    out << std::fixed << std::setprecision(6) << value;
    std::string text = out.str();
    while (text.size() > 1 && text.back() == '0') {
        text.pop_back();
    }
    if (!text.empty() && text.back() == '.') {
        text.push_back('0');
    }
    return text;
}

std::string dump_json(const Json& json, int indent = -1, int depth = 0) {
    switch (json.type) {
        case Json::Type::Null:
            return "null";
        case Json::Type::Bool:
            return json.bool_value ? "true" : "false";
        case Json::Type::Number:
            return format_number(json.number_value);
        case Json::Type::String:
            return "\"" + escape_json_string(json.string_value) + "\"";
        case Json::Type::Array: {
            if (json.array_value.empty()) {
                return "[]";
            }
            std::ostringstream out;
            out << "[";
            for (std::size_t i = 0; i < json.array_value.size(); ++i) {
                if (i > 0) {
                    out << ",";
                }
                if (indent >= 0) {
                    out << "\n" << std::string((depth + 1) * indent, ' ');
                }
                out << dump_json(json.array_value[i], indent, depth + 1);
            }
            if (indent >= 0) {
                out << "\n" << std::string(depth * indent, ' ');
            }
            out << "]";
            return out.str();
        }
        case Json::Type::Object: {
            if (json.object_value.empty()) {
                return "{}";
            }
            std::ostringstream out;
            out << "{";
            std::size_t i = 0;
            for (const auto& [key, value] : json.object_value) {
                if (i++ > 0) {
                    out << ",";
                }
                if (indent >= 0) {
                    out << "\n" << std::string((depth + 1) * indent, ' ');
                }
                out << "\"" << escape_json_string(key) << "\":";
                if (indent >= 0) {
                    out << " ";
                }
                out << dump_json(value, indent, depth + 1);
            }
            if (indent >= 0) {
                out << "\n" << std::string(depth * indent, ' ');
            }
            out << "}";
            return out.str();
        }
    }
    return "null";
}

std::string sha256_hex(const std::vector<std::uint8_t>& bytes) {
    constexpr std::array<std::uint32_t, 64> k = {
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    };
    auto rotr = [](std::uint32_t value, std::uint32_t bits) {
        return (value >> bits) | (value << (32 - bits));
    };
    std::array<std::uint32_t, 8> h = {
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    };
    std::vector<std::uint8_t> data = bytes;
    const std::uint64_t bit_len = static_cast<std::uint64_t>(data.size()) * 8;
    data.push_back(0x80);
    while ((data.size() % 64) != 56) {
        data.push_back(0);
    }
    for (int shift = 56; shift >= 0; shift -= 8) {
        data.push_back(static_cast<std::uint8_t>((bit_len >> shift) & 0xff));
    }

    for (std::size_t chunk = 0; chunk < data.size(); chunk += 64) {
        std::array<std::uint32_t, 64> w{};
        for (std::size_t i = 0; i < 16; ++i) {
            const std::size_t j = chunk + i * 4;
            w[i] = (static_cast<std::uint32_t>(data[j]) << 24)
                | (static_cast<std::uint32_t>(data[j + 1]) << 16)
                | (static_cast<std::uint32_t>(data[j + 2]) << 8)
                | static_cast<std::uint32_t>(data[j + 3]);
        }
        for (std::size_t i = 16; i < 64; ++i) {
            const std::uint32_t s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
            const std::uint32_t s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }
        std::uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
        for (std::size_t i = 0; i < 64; ++i) {
            const std::uint32_t s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
            const std::uint32_t ch = (e & f) ^ ((~e) & g);
            const std::uint32_t temp1 = hh + s1 + ch + k[i] + w[i];
            const std::uint32_t s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
            const std::uint32_t maj = (a & b) ^ (a & c) ^ (b & c);
            const std::uint32_t temp2 = s0 + maj;
            hh = g;
            g = f;
            f = e;
            e = d + temp1;
            d = c;
            c = b;
            b = a;
            a = temp1 + temp2;
        }
        h[0] += a;
        h[1] += b;
        h[2] += c;
        h[3] += d;
        h[4] += e;
        h[5] += f;
        h[6] += g;
        h[7] += hh;
    }

    std::ostringstream out;
    for (std::uint32_t word : h) {
        out << std::hex << std::setw(8) << std::setfill('0') << word;
    }
    return out.str();
}

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
    std::string engine = "fallback-stl-parser";
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

MeshStats load_stl_fallback(const fs::path& path) {
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

#if CADASTROPHE_WITH_OPEN3D
MeshStats load_stl_open3d(const fs::path& path) {
    MeshStats stats;
    const std::vector<std::uint8_t> bytes = read_binary_file(path);
    stats.bytes = bytes.size();
    stats.sha256 = sha256_hex(bytes);
    stats.engine = "open3d";
    stats.topology_approximate = false;

    open3d::geometry::TriangleMesh mesh;
    if (!open3d::io::ReadTriangleMesh(path.string(), mesh)) {
        stats.error = "Open3D failed to read STL.";
        return stats;
    }
    stats.raw_vertices = mesh.vertices_.size();
    stats.raw_triangles = mesh.triangles_.size();
    mesh.RemoveDuplicatedVertices();
    mesh.RemoveDegenerateTriangles();
    mesh.RemoveUnreferencedVertices();
    stats.unique_vertices = mesh.vertices_.size();
    stats.triangles_after_cleanup = mesh.triangles_.size();
    stats.degenerate_triangles = stats.raw_triangles > stats.triangles_after_cleanup
        ? stats.raw_triangles - stats.triangles_after_cleanup
        : 0;
    if (mesh.vertices_.empty() || mesh.triangles_.empty()) {
        stats.error = "Open3D loaded an empty mesh after cleanup.";
        return stats;
    }
    const auto bbox = mesh.GetAxisAlignedBoundingBox();
    const auto extent = bbox.GetExtent();
    stats.bbox = {extent(0), extent(1), extent(2)};
    stats.bbox_volume = stats.bbox[0] * stats.bbox[1] * stats.bbox[2];
    stats.edge_manifold_closed = mesh.IsEdgeManifold(false);
    stats.edge_manifold_with_boundary = mesh.IsEdgeManifold(true);
    stats.vertex_manifold = mesh.IsVertexManifold();
    stats.orientable = mesh.IsOrientable();
    stats.self_intersecting = mesh.IsSelfIntersecting();
    try {
        stats.solid_volume = mesh.GetVolume();
    } catch (...) {
        stats.solid_volume = 0.0;
    }
    stats.loaded = true;
    return stats;
}
#endif

MeshStats load_mesh(const fs::path& path) {
#if CADASTROPHE_WITH_OPEN3D
    return load_stl_open3d(path);
#else
    return load_stl_fallback(path);
#endif
}

std::array<double, 3> normalize_aspect(const std::array<double, 3>& values) {
    const double max_value = std::max({values[0], values[1], values[2]});
    if (max_value <= 0.0) {
        return {0.0, 0.0, 0.0};
    }
    return {values[0] / max_value, values[1] / max_value, values[2] / max_value};
}

Json number_array(const std::array<double, 3>& values) {
    return Json::array({Json::number(values[0]), Json::number(values[1]), Json::number(values[2])});
}

Json check_json(
    const std::string& name,
    bool passed,
    const std::string& severity,
    const std::string& message,
    std::map<std::string, Json> details = {}
) {
    std::map<std::string, Json> object{
        {"message", Json::string(message)},
        {"name", Json::string(name)},
        {"passed", Json::boolean(passed)},
        {"severity", Json::string(severity)},
    };
    for (auto& [key, value] : details) {
        object.emplace(key, std::move(value));
    }
    return Json::object(std::move(object));
}

std::string reason_for_failed_check(const std::vector<Json>& checks) {
    for (const Json& check : checks) {
        if (!get_bool(check, "passed", false)) {
            return get_string(check, "name") + "_failed";
        }
    }
    return "";
}

bool has_error_diagnostic(const Json& diagnostics) {
    if (get_bool(diagnostics, "ok", false) == false) {
        return true;
    }
    const Json* items = diagnostics.get("items");
    if (!items || !items->is_array()) {
        return false;
    }
    for (const Json& item : items->array_value) {
        if (get_string(item, "severity") == "error") {
            return true;
        }
    }
    return false;
}

std::string metadata_string(const Json& manifest, const std::string& key) {
    if (const Json* value = manifest.get(key); value && value->is_string()) {
        return value->string_value;
    }
    if (const Json* metadata = manifest.get("metadata"); metadata && metadata->is_object()) {
        if (const Json* value = metadata->get(key); value && value->is_string()) {
            return value->string_value;
        }
    }
    return "";
}

double metadata_number(const Json& manifest, const std::string& key, double fallback = 0.0) {
    if (const Json* value = manifest.get(key); value && value->is_number()) {
        return value->number_value;
    }
    if (const Json* metadata = manifest.get("metadata"); metadata && metadata->is_object()) {
        if (const Json* value = metadata->get(key); value && value->is_number()) {
            return value->number_value;
        }
    }
    return fallback;
}

std::string source_text_from_input(const Json& input, const Json& manifest, const fs::path& base_dir) {
    std::string source = get_string(input, "sourceText");
    if (!source.empty()) {
        return source;
    }
    const std::string source_path = get_string(input, "sourcePath");
    if (!source_path.empty()) {
        return read_text_file(resolve_path(base_dir, source_path));
    }
    source = metadata_string(manifest, "sourceText");
    if (!source.empty()) {
        return source;
    }
    const std::string annotation = metadata_string(manifest, "mainComponentAnnotation");
    if (!annotation.empty()) {
        return annotation;
    }
    const std::string main_component = metadata_string(manifest, "mainComponent");
    if (!main_component.empty()) {
        return "// @main_component " + main_component;
    }
    return "";
}

Json evaluate(const Json& input, const fs::path& base_dir) {
    Json plan = load_json_field_or_path(input, base_dir, "plan", "planPath");
    Json manifest = load_json_field_or_path(input, base_dir, "artifactManifest", "artifactManifestPath");
    Json diagnostics = load_json_field_or_path(input, base_dir, "runtimeDiagnostics", "runtimeDiagnosticsPath");

    const std::string run_id = get_string(input, "runId");
    const std::string revision_id = get_string(input, "revisionId");
    std::string artifact_id = get_string(input, "artifactId");
    if (artifact_id.empty()) {
        artifact_id = get_string(manifest, "id");
    }
    const std::string stl_path_text = get_string(input, "stlPath");
    if (stl_path_text.empty()) {
        throw std::runtime_error("input.stlPath is required");
    }
    const fs::path stl_path = resolve_path(base_dir, stl_path_text);

    const Json* main_component = plan.get("mainComponent");
    if (!main_component || !main_component->is_object()) {
        throw std::runtime_error("plan.mainComponent is required");
    }
    const std::string main_name = get_string(*main_component, "name");
    if (main_name.empty()) {
        throw std::runtime_error("plan.mainComponent.name is required");
    }
    const Json* expected_aspect = plan.get("expectedAspectRatio");
    if (!expected_aspect || !expected_aspect->is_object()) {
        throw std::runtime_error("plan.expectedAspectRatio is required");
    }
    const std::array<double, 3> expected = {
        get_number(*expected_aspect, "x"),
        get_number(*expected_aspect, "y"),
        get_number(*expected_aspect, "z"),
    };
    const double tolerance = get_number(*expected_aspect, "tolerance", 0.3);
    const Json* runtime_constraints = plan.get("runtimeConstraints");
    std::string required_annotation = "// @main_component " + main_name;
    if (runtime_constraints && runtime_constraints->is_object()) {
        required_annotation = get_string(*runtime_constraints, "mainComponentAnnotation", required_annotation);
    }

    const std::string source_text = source_text_from_input(input, manifest, base_dir);
    const bool main_component_passed = source_text.find(required_annotation) != std::string::npos;

    std::vector<Json> checks;
    checks.push_back(check_json(
        "main_component",
        main_component_passed,
        main_component_passed ? "info" : "error",
        main_component_passed
            ? "Final source declares the Plan main component convention."
            : "Final source or artifact metadata does not declare the Plan main component convention.",
        {
            {"expectedAnnotation", Json::string(required_annotation)},
            {"mainComponent", Json::string(main_name)},
        }
    ));

    const bool runtime_passed = !has_error_diagnostic(diagnostics);
    checks.push_back(check_json(
        "runtime_diagnostics",
        runtime_passed,
        runtime_passed ? "info" : "error",
        runtime_passed ? "Preview and export diagnostics are clean." : "Runtime diagnostics contain errors.",
        {
            {"ok", Json::boolean(get_bool(diagnostics, "ok", false))},
            {"elapsedMs", Json::number(get_number(diagnostics, "elapsedMs", get_number(diagnostics, "elapsed_ms", 0.0)))},
        }
    ));

    bool file_exists = fs::exists(stl_path);
    MeshStats mesh;
    if (file_exists) {
        mesh = load_mesh(stl_path);
    } else {
        mesh.error = "STL file does not exist.";
    }

    const double expected_bytes = metadata_number(manifest, "bytes", -1.0);
    const std::string expected_sha256 = metadata_string(manifest, "sha256");
    const bool bytes_passed = expected_bytes < 0.0 || static_cast<std::uint64_t>(expected_bytes) == mesh.bytes;
    const bool sha_passed = expected_sha256.empty() || expected_sha256 == mesh.sha256;
    const bool manifest_passed = file_exists && bytes_passed && sha_passed;
    checks.push_back(check_json(
        "artifact_manifest",
        manifest_passed,
        manifest_passed ? "info" : "error",
        manifest_passed ? "STL artifact exists and manifest hash/size match." : "STL artifact manifest validation failed.",
        {
            {"actualBytes", Json::number(static_cast<double>(mesh.bytes))},
            {"actualSha256", Json::string(mesh.sha256)},
            {"expectedBytes", expected_bytes < 0.0 ? Json::null() : Json::number(expected_bytes)},
            {"expectedSha256", expected_sha256.empty() ? Json::null() : Json::string(expected_sha256)},
            {"stlPath", Json::string(stl_path.string())},
        }
    ));

    checks.push_back(check_json(
        "stl_load",
        mesh.loaded,
        mesh.loaded ? "info" : "error",
        mesh.loaded ? "STL mesh loaded." : mesh.error,
        {
            {"engine", Json::string(mesh.engine)},
            {"error", mesh.error.empty() ? Json::null() : Json::string(mesh.error)},
        }
    ));

    const bool non_empty = mesh.loaded && mesh.unique_vertices > 0 && mesh.triangles_after_cleanup > 0 && mesh.bbox_volume > 0.0;
    checks.push_back(check_json(
        "mesh_non_empty",
        non_empty,
        non_empty ? "info" : "error",
        non_empty ? "Mesh has positive vertices, triangles, and bounding box volume." : "Mesh is empty or has zero bounding box volume.",
        {
            {"bbox", number_array(mesh.bbox)},
            {"bboxVolume", Json::number(mesh.bbox_volume)},
            {"triangles", Json::number(static_cast<double>(mesh.triangles_after_cleanup))},
            {"vertices", Json::number(static_cast<double>(mesh.unique_vertices))},
        }
    ));

    const bool cleanup_passed = mesh.loaded && mesh.triangles_after_cleanup > 0;
    checks.push_back(check_json(
        "mesh_cleanup",
        cleanup_passed,
        cleanup_passed ? "info" : "error",
        cleanup_passed ? "Duplicated/degenerate triangle cleanup leaves a valid mesh." : "Cleanup removed all usable triangles.",
        {
            {"degenerateTrianglesRemoved", Json::number(static_cast<double>(mesh.degenerate_triangles))},
            {"rawTriangles", Json::number(static_cast<double>(mesh.raw_triangles))},
            {"trianglesAfterCleanup", Json::number(static_cast<double>(mesh.triangles_after_cleanup))},
            {"uniqueVerticesAfterCleanup", Json::number(static_cast<double>(mesh.unique_vertices))},
        }
    ));

    const bool topology_passed = mesh.loaded
        && mesh.edge_manifold_closed
        && mesh.vertex_manifold
        && mesh.orientable
        && !mesh.self_intersecting
        && mesh.solid_volume > 0.0;
    checks.push_back(check_json(
        "topology",
        topology_passed,
        topology_passed ? "info" : "error",
        topology_passed ? "Solid/topology checks passed." : "Solid/topology checks failed.",
        {
            {"edgeManifoldClosed", Json::boolean(mesh.edge_manifold_closed)},
            {"edgeManifoldWithBoundary", Json::boolean(mesh.edge_manifold_with_boundary)},
            {"hasVolume", Json::boolean(mesh.solid_volume > 0.0)},
            {"isSelfIntersecting", Json::boolean(mesh.self_intersecting)},
            {"isVertexManifold", Json::boolean(mesh.vertex_manifold)},
            {"orientable", Json::boolean(mesh.orientable)},
            {"topologyApproximate", Json::boolean(mesh.topology_approximate)},
            {"volume", Json::number(mesh.solid_volume)},
        }
    ));

    const std::array<double, 3> expected_normalized = normalize_aspect(expected);
    const std::array<double, 3> actual_normalized = normalize_aspect(mesh.bbox);
    const std::array<double, 3> deltas = {
        std::fabs(expected_normalized[0] - actual_normalized[0]),
        std::fabs(expected_normalized[1] - actual_normalized[1]),
        std::fabs(expected_normalized[2] - actual_normalized[2]),
    };
    const double max_delta = std::max({deltas[0], deltas[1], deltas[2]});
    const bool aspect_passed = mesh.loaded && max_delta <= tolerance;
    checks.push_back(check_json(
        "aspect_ratio",
        aspect_passed,
        aspect_passed ? "info" : "error",
        aspect_passed ? "Bounding box aspect ratio is within tolerance." : "Bounding box aspect ratio is outside tolerance.",
        {
            {"actual", Json::object({{"x", Json::number(mesh.bbox[0])}, {"y", Json::number(mesh.bbox[1])}, {"z", Json::number(mesh.bbox[2])}})},
            {"actualNormalized", number_array(actual_normalized)},
            {"axisDeltas", number_array(deltas)},
            {"expected", Json::object({{"tolerance", Json::number(tolerance)}, {"x", Json::number(expected[0])}, {"y", Json::number(expected[1])}, {"z", Json::number(expected[2])}})},
            {"expectedNormalized", number_array(expected_normalized)},
            {"maxAxisDelta", Json::number(max_delta)},
        }
    ));

    bool passed = true;
    for (const Json& check : checks) {
        passed = passed && get_bool(check, "passed", false);
    }

    std::map<std::string, Json> report{
        {"artifactId", Json::string(artifact_id)},
        {"checks", Json::array(checks)},
        {"contractType", Json::string("cadastrophe.structural_report.v1")},
        {"mainComponent", Json::string(main_name)},
        {"passed", Json::boolean(passed)},
        {"revisionId", Json::string(revision_id)},
        {"runId", Json::string(run_id)},
    };
    if (!passed) {
        const std::string reason = reason_for_failed_check(checks);
        report.emplace(
            "failureReport",
            Json::object({
                {"contractType", Json::string("cadastrophe.failure_report.v1")},
                {"nextAction", Json::string("refine_plan_or_source")},
                {"reason", Json::string(reason.empty() ? "structural_anchor_failed" : reason)},
            })
        );
    }
    return Json::object(std::move(report));
}

std::string read_stdin() {
    std::ostringstream buffer;
    buffer << std::cin.rdbuf();
    return buffer.str();
}

void print_help() {
    std::cout
        << "cadastrophe-structural-anchor\n"
        << "Usage: cadastrophe-structural-anchor [--input report-input.json] [--pretty]\n\n"
        << "Input JSON fields: runId, revisionId, artifactId, plan|planPath, stlPath,\n"
        << "artifactManifest|artifactManifestPath, runtimeDiagnostics|runtimeDiagnosticsPath,\n"
        << "and sourceText|sourcePath or manifest metadata for the @main_component check.\n";
}

}  // namespace

int main(int argc, char** argv) {
    try {
        std::string input_path;
        bool pretty = false;
        for (int i = 1; i < argc; ++i) {
            std::string arg = argv[i];
            if (arg == "--help" || arg == "-h") {
                print_help();
                return 0;
            }
            if (arg == "--pretty") {
                pretty = true;
                continue;
            }
            if (arg == "--input") {
                if (i + 1 >= argc) {
                    throw std::runtime_error("--input requires a path");
                }
                input_path = argv[++i];
                continue;
            }
            throw std::runtime_error("unknown argument: " + arg);
        }

        fs::path base_dir = fs::current_path();
        std::string input_text;
        if (!input_path.empty()) {
            fs::path path(input_path);
            if (!path.is_absolute()) {
                path = fs::weakly_canonical(fs::current_path() / path);
            }
            base_dir = path.parent_path();
            input_text = read_text_file(path);
        } else {
            input_text = read_stdin();
        }
        Json input = JsonParser(input_text).parse();
        Json report = evaluate(input, base_dir);
        std::cout << dump_json(report, pretty ? 2 : -1) << "\n";
        return 0;
    } catch (const std::exception& error) {
        Json envelope = Json::object({
            {"contractType", Json::string("cadastrophe.structural_anchor_error.v1")},
            {"error", Json::string(error.what())},
        });
        std::cerr << dump_json(envelope) << "\n";
        return 2;
    }
}
