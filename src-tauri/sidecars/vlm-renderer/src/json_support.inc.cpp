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
    bool is_string() const { return type == Type::String; }
    bool is_number() const { return type == Type::Number; }
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
    bool peek(char ch) const { return pos_ < input_.size() && input_[pos_] == ch; }
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

std::string get_string(const Json& json, const std::string& key, const std::string& default_value = "") {
    const Json* value = json.get(key);
    return value && value->is_string() ? value->string_value : default_value;
}

double get_number(const Json& json, const std::string& key, double default_value) {
    const Json* value = json.get(key);
    return value && value->is_number() ? value->number_value : default_value;
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
    if (std::fabs(value - std::round(value)) < 0.0000005) {
        return std::to_string(static_cast<long long>(std::llround(value)));
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

std::string dump_json(const Json& json) {
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
            std::ostringstream out;
            out << "[";
            for (std::size_t i = 0; i < json.array_value.size(); ++i) {
                if (i > 0) {
                    out << ",";
                }
                out << dump_json(json.array_value[i]);
            }
            out << "]";
            return out.str();
        }
        case Json::Type::Object: {
            std::ostringstream out;
            out << "{";
            std::size_t i = 0;
            for (const auto& [key, value] : json.object_value) {
                if (i++ > 0) {
                    out << ",";
                }
                out << "\"" << escape_json_string(key) << "\":" << dump_json(value);
            }
            out << "}";
            return out.str();
        }
    }
    return "null";
}
