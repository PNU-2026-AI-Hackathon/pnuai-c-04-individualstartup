struct View {
    const char* name;
    Vec3 camera;
    int col;
    int row;
};

std::vector<View> views_for_mode(const std::string& view_mode) {
    if (view_mode == "1-view") {
        return {{"Front", {0, -1, 0}, 0, 0}};
    }
    if (view_mode == "3-view") {
        return {{"Front-Right-Top", {1, -1, 1}, 0, 0}, {"Back-Top", {0, 1, 1}, 1, 0}, {"Front", {0, -1, 0}, 2, 0}};
    }
    return {
        {"Front-Left-Top", {-1, -1, 1}, 0, 0},
        {"Front", {0, -1, 0}, 1, 0},
        {"Front-Right-Top", {1, -1, 1}, 2, 0},
        {"Left", {-1, 0, 0}, 0, 1},
        {"Top", {0, 0, 1}, 1, 1},
        {"Right", {1, 0, 0}, 2, 1},
        {"Bottom", {0, 0, -1}, 0, 2},
        {"Back", {0, 1, 0}, 1, 2},
        {"Back-Right-Top", {1, 1, 1}, 2, 2},
    };
}

std::string normalize_view_mode(std::string raw) {
    std::transform(raw.begin(), raw.end(), raw.begin(), [](unsigned char ch) {
        return static_cast<char>(std::tolower(ch));
    });
    std::replace(raw.begin(), raw.end(), '_', '-');
    if (raw == "1" || raw == "one" || raw == "one-view" || raw == "1-view") {
        return "1-view";
    }
    if (raw == "3" || raw == "three" || raw == "three-view" || raw == "3-view") {
        return "3-view";
    }
    return "9-view";
}

void draw_line(Image& image, int x0, int y0, int x1, int y1, Color color) {
    const int dx = std::abs(x1 - x0);
    const int sx = x0 < x1 ? 1 : -1;
    const int dy = -std::abs(y1 - y0);
    const int sy = y0 < y1 ? 1 : -1;
    int error = dx + dy;
    while (true) {
        image.set(x0, y0, color);
        if (x0 == x1 && y0 == y1) {
            break;
        }
        const int e2 = 2 * error;
        if (e2 >= dy) {
            error += dy;
            x0 += sx;
        }
        if (e2 <= dx) {
            error += dx;
            y0 += sy;
        }
    }
}

double edge_function(double ax, double ay, double bx, double by, double px, double py) {
    return (px - ax) * (by - ay) - (py - ay) * (bx - ax);
}

void fill_triangle(Image& image, std::array<double, 2> a, std::array<double, 2> b, std::array<double, 2> c, Color color) {
    const int min_x = std::max(0, static_cast<int>(std::floor(std::min({a[0], b[0], c[0]}))));
    const int max_x = std::min(image.width - 1, static_cast<int>(std::ceil(std::max({a[0], b[0], c[0]}))));
    const int min_y = std::max(0, static_cast<int>(std::floor(std::min({a[1], b[1], c[1]}))));
    const int max_y = std::min(image.height - 1, static_cast<int>(std::ceil(std::max({a[1], b[1], c[1]}))));
    const double area = edge_function(a[0], a[1], b[0], b[1], c[0], c[1]);
    if (std::fabs(area) <= 1e-9) {
        return;
    }
    for (int y = min_y; y <= max_y; ++y) {
        for (int x = min_x; x <= max_x; ++x) {
            const double px = x + 0.5;
            const double py = y + 0.5;
            const double w0 = edge_function(b[0], b[1], c[0], c[1], px, py);
            const double w1 = edge_function(c[0], c[1], a[0], a[1], px, py);
            const double w2 = edge_function(a[0], a[1], b[0], b[1], px, py);
            if ((w0 >= 0 && w1 >= 0 && w2 >= 0) || (w0 <= 0 && w1 <= 0 && w2 <= 0)) {
                image.set(x, y, color);
            }
        }
    }
}

std::array<double, 2> project_point(const Vec3& point, const Vec3& right, const Vec3& up, double scale, int width, int height) {
    const double x = dot(point, right) * scale + width * 0.5;
    const double y = -dot(point, up) * scale + height * 0.5;
    return {x, y};
}

std::vector<Triangle> normalize_mesh(std::vector<Triangle> triangles) {
    Vec3 min_v{1e300, 1e300, 1e300};
    Vec3 max_v{-1e300, -1e300, -1e300};
    auto include = [&](const Vec3& v) {
        min_v.x = std::min(min_v.x, v.x);
        min_v.y = std::min(min_v.y, v.y);
        min_v.z = std::min(min_v.z, v.z);
        max_v.x = std::max(max_v.x, v.x);
        max_v.y = std::max(max_v.y, v.y);
        max_v.z = std::max(max_v.z, v.z);
    };
    for (const Triangle& triangle : triangles) {
        include(triangle.a);
        include(triangle.b);
        include(triangle.c);
    }
    const Vec3 center = (min_v + max_v) * 0.5;
    double radius = 0.0;
    for (const Triangle& triangle : triangles) {
        radius = std::max({radius, norm(triangle.a - center), norm(triangle.b - center), norm(triangle.c - center)});
    }
    const double inv_radius = radius <= 1e-9 ? 1.0 : 1.0 / radius;
    for (Triangle& triangle : triangles) {
        triangle.a = (triangle.a - center) * inv_radius;
        triangle.b = (triangle.b - center) * inv_radius;
        triangle.c = (triangle.c - center) * inv_radius;
    }
    return triangles;
}

Image render_view(const std::vector<Triangle>& triangles, const View& view, int width, int height) {
    Image image(width, height, {248, 250, 252});
    const Vec3 camera = normalized(view.camera);
    const Vec3 forward = camera * -1.0;
    const Vec3 world_up = std::fabs(dot(camera, {0, 0, 1})) > 0.92 ? Vec3{0, 1, 0} : Vec3{0, 0, 1};
    const Vec3 right = normalized(cross(world_up, forward));
    const Vec3 up = normalized(cross(forward, right));
    const double scale = std::min(width, height) * 0.38;

    struct Projected {
        std::array<double, 2> a;
        std::array<double, 2> b;
        std::array<double, 2> c;
        double depth;
        double shade;
    };
    std::vector<Projected> projected;
    for (const Triangle& triangle : triangles) {
        const Vec3 normal = normalized(cross(triangle.b - triangle.a, triangle.c - triangle.a));
        const double shade = 0.55 + 0.35 * std::max(0.0, dot(normalized({-0.4, -0.7, 1.0}), normal));
        projected.push_back({
            project_point(triangle.a, right, up, scale, width, height),
            project_point(triangle.b, right, up, scale, width, height),
            project_point(triangle.c, right, up, scale, width, height),
            (dot(triangle.a, forward) + dot(triangle.b, forward) + dot(triangle.c, forward)) / 3.0,
            shade,
        });
    }
    std::sort(projected.begin(), projected.end(), [](const Projected& left, const Projected& right) {
        return left.depth < right.depth;
    });
    for (const Projected& triangle : projected) {
        const auto tint = static_cast<std::uint8_t>(std::clamp(205.0 * triangle.shade, 110.0, 230.0));
        fill_triangle(image, triangle.a, triangle.b, triangle.c, {tint, static_cast<std::uint8_t>(tint - 24), 184});
        draw_line(image, static_cast<int>(triangle.a[0]), static_cast<int>(triangle.a[1]), static_cast<int>(triangle.b[0]), static_cast<int>(triangle.b[1]), {93, 93, 115});
        draw_line(image, static_cast<int>(triangle.b[0]), static_cast<int>(triangle.b[1]), static_cast<int>(triangle.c[0]), static_cast<int>(triangle.c[1]), {93, 93, 115});
        draw_line(image, static_cast<int>(triangle.c[0]), static_cast<int>(triangle.c[1]), static_cast<int>(triangle.a[0]), static_cast<int>(triangle.a[1]), {93, 93, 115});
    }
    return image;
}

const std::array<std::string, 7>& glyph(char ch) {
    static const std::array<std::string, 7> blank = {"00000", "00000", "00000", "00000", "00000", "00000", "00000"};
    static const std::map<char, std::array<std::string, 7>> glyphs = {
        {'A', {"01110", "10001", "10001", "11111", "10001", "10001", "10001"}},
        {'B', {"11110", "10001", "10001", "11110", "10001", "10001", "11110"}},
        {'C', {"01111", "10000", "10000", "10000", "10000", "10000", "01111"}},
        {'F', {"11111", "10000", "10000", "11110", "10000", "10000", "10000"}},
        {'G', {"01111", "10000", "10000", "10111", "10001", "10001", "01111"}},
        {'H', {"10001", "10001", "10001", "11111", "10001", "10001", "10001"}},
        {'I', {"11111", "00100", "00100", "00100", "00100", "00100", "11111"}},
        {'K', {"10001", "10010", "10100", "11000", "10100", "10010", "10001"}},
        {'L', {"10000", "10000", "10000", "10000", "10000", "10000", "11111"}},
        {'M', {"10001", "11011", "10101", "10101", "10001", "10001", "10001"}},
        {'N', {"10001", "11001", "10101", "10011", "10001", "10001", "10001"}},
        {'O', {"01110", "10001", "10001", "10001", "10001", "10001", "01110"}},
        {'P', {"11110", "10001", "10001", "11110", "10000", "10000", "10000"}},
        {'R', {"11110", "10001", "10001", "11110", "10100", "10010", "10001"}},
        {'T', {"11111", "00100", "00100", "00100", "00100", "00100", "00100"}},
        {'U', {"10001", "10001", "10001", "10001", "10001", "10001", "01110"}},
        {'W', {"10001", "10001", "10001", "10101", "10101", "10101", "01010"}},
        {'-', {"00000", "00000", "00000", "11111", "00000", "00000", "00000"}},
    };
    auto found = glyphs.find(ch);
    return found == glyphs.end() ? blank : found->second;
}

void fill_rect(Image& image, int x, int y, int width, int height, Color color) {
    for (int yy = y; yy < y + height; ++yy) {
        for (int xx = x; xx < x + width; ++xx) {
            image.set(xx, yy, color);
        }
    }
}

void draw_text(Image& image, int x, int y, const std::string& text, Color color, int scale = 2) {
    int cursor = x;
    for (char raw : text) {
        const char ch = static_cast<char>(std::toupper(static_cast<unsigned char>(raw)));
        const auto& rows = glyph(ch);
        for (int row = 0; row < 7; ++row) {
            for (int col = 0; col < 5; ++col) {
                if (rows[static_cast<std::size_t>(row)][static_cast<std::size_t>(col)] == '1') {
                    fill_rect(image, cursor + col * scale, y + row * scale, scale, scale, color);
                }
            }
        }
        cursor += 6 * scale;
    }
}

Image create_grid(const std::vector<Triangle>& triangles, const std::vector<View>& views, int cell_width, int cell_height, int cols, int rows) {
    Image grid(cell_width * cols, cell_height * rows, {255, 255, 255});
    for (const View& view : views) {
        Image rendered = render_view(triangles, view, cell_width, cell_height);
        const int ox = view.col * cell_width;
        const int oy = view.row * cell_height;
        for (int y = 0; y < cell_height; ++y) {
            for (int x = 0; x < cell_width; ++x) {
                grid.set(ox + x, oy + y, rendered.get(x, y));
            }
        }
        fill_rect(grid, ox + 10, oy + 10, std::min(cell_width - 20, static_cast<int>(std::strlen(view.name)) * 12 + 12), 24, {255, 255, 255});
        draw_text(grid, ox + 16, oy + 16, view.name, {17, 24, 39}, 2);
    }
    return grid;
}
