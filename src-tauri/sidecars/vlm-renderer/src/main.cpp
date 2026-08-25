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
#include <map>
#include <numeric>
#include <sstream>
#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace fs = std::filesystem;

namespace {

#include "json_support.inc.cpp"
#include "png.inc.cpp"
#include "sha256.inc.cpp"
#include "mesh.inc.cpp"
#include "render.inc.cpp"
#include "input.inc.cpp"

}  // namespace

int main(int argc, char** argv) {
    try {
        const Json input = load_input(argc, argv);
        const fs::path stl_path = get_string(input, "stlPath");
        if (stl_path.empty()) {
            throw std::runtime_error("missing stlPath");
        }
        const fs::path output_dir = get_string(input, "outputDirectory", ".");
        fs::create_directories(output_dir);
        const std::string view_mode = normalize_view_mode(get_string(input, "viewMode", "9-view"));
        const int cell_width = resolution_component(input, "width", 512);
        const int cell_height = resolution_component(input, "height", 512);
        const std::vector<View> views = views_for_mode(view_mode);
        const int cols = view_mode == "1-view" ? 1 : 3;
        const int rows = view_mode == "9-view" ? 3 : 1;

        const std::vector<Triangle> triangles = normalize_mesh(load_mesh_triangles(stl_path));
        const Image grid = create_grid(triangles, views, cell_width, cell_height, cols, rows);
        const std::vector<std::uint8_t> png = encode_png(grid);
        const fs::path output_path = fs::absolute(output_dir / "vlm-render-grid.png");
        std::ofstream out(output_path, std::ios::binary);
        out.write(reinterpret_cast<const char*>(png.data()), static_cast<std::streamsize>(png.size()));
        out.close();

        std::vector<Json> view_names;
        for (const View& view : views) {
            view_names.push_back(Json::string(view.name));
        }
        const Json manifest = Json::object({
            {"artifactId", Json::string(get_string(input, "artifactId"))},
            {"bytes", Json::number(static_cast<double>(png.size()))},
            {"contractType", Json::string("cadgen-ax.vlm_render_manifest.v1")},
            {"format", Json::string("png")},
            {"path", Json::string(output_path.string())},
            {"renderer", Json::string("cadgen-ax-vlm-renderer")},
            {"rendererEngine", Json::string("native-cpp-rasterizer")},
            {"resolution", Json::object({{"width", Json::number(cell_width)}, {"height", Json::number(cell_height)}})},
            {"revisionId", Json::string(get_string(input, "revisionId"))},
            {"runId", Json::string(get_string(input, "runId"))},
            {"sha256", Json::string(sha256_hex(png))},
            {"sourceArtifactId", Json::string(get_string(input, "artifactId"))},
            {"sourceArtifactSha256", Json::string(get_string(input, "sourceArtifactSha256"))},
            {"sourceHash", Json::string(get_string(input, "sourceHash"))},
            {"viewMode", Json::string(view_mode)},
            {"views", Json::array(std::move(view_names))},
        });
        std::cout << dump_json(manifest) << "\n";
        return 0;
    } catch (const std::exception& error) {
        std::cerr << "cadgen-ax-vlm-renderer failed: " << error.what() << "\n";
        return 1;
    }
}
