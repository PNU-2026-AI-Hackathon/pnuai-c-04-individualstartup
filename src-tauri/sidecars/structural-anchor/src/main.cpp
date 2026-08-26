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

#include "mesh_validator.h"

namespace fs = std::filesystem;

namespace {

#include "json_support.inc.cpp"
#include "sha256.inc.cpp"
#include "mesh_stats.inc.cpp"
#include "evaluate.inc.cpp"
#include "cli_support.inc.cpp"

}  // namespace

int main(int argc, char** argv) {
    try {
        std::string input_path;
        std::string input_stl_path;
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
            if (arg == "--input-stl") {
                if (i + 1 >= argc) {
                    throw std::runtime_error("--input-stl requires a path");
                }
                input_stl_path = argv[++i];
                continue;
            }
            throw std::runtime_error("unknown argument: " + arg);
        }

        if (!input_path.empty() && !input_stl_path.empty()) {
            throw std::runtime_error("--input and --input-stl cannot be used together");
        }

        Json report;
        if (!input_stl_path.empty()) {
            fs::path stl_path(input_stl_path);
            if (!stl_path.is_absolute()) {
                stl_path = fs::weakly_canonical(fs::current_path() / stl_path);
            }
            report = evaluate_stl(stl_path);
            std::cout << dump_json(report, pretty ? 2 : -1) << "\n";
            return 0;
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
        report = evaluate(input, base_dir);
        std::cout << dump_json(report, pretty ? 2 : -1) << "\n";
        return 0;
    } catch (const std::exception& error) {
        Json envelope = Json::object({
            {"contractType", Json::string("cadgen-ax.structural_anchor_error.v1")},
            {"error", Json::string(error.what())},
        });
        std::cerr << dump_json(envelope) << "\n";
        return 2;
    }
}
