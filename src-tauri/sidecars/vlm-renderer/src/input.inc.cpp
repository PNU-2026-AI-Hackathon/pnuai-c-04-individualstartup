std::string read_stdin() {
    std::ostringstream buffer;
    buffer << std::cin.rdbuf();
    return buffer.str();
}

Json load_input(int argc, char** argv) {
    for (int index = 1; index < argc; ++index) {
        const std::string arg = argv[index];
        if (arg == "--input" && index + 1 < argc) {
            return JsonParser(read_text_file(argv[index + 1])).parse();
        }
    }
    return JsonParser(read_stdin()).parse();
}

int resolution_component(const Json& input, const std::string& key, int default_value) {
    if (const Json* resolution = input.get("resolution")) {
        return static_cast<int>(std::max(64.0, get_number(*resolution, key, default_value)));
    }
    return default_value;
}
