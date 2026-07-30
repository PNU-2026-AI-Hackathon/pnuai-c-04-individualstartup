std::uint32_t crc32(const std::vector<std::uint8_t>& bytes) {
    std::uint32_t crc = 0xffffffffU;
    for (std::uint8_t byte : bytes) {
        crc ^= byte;
        for (int bit = 0; bit < 8; ++bit) {
            crc = (crc >> 1) ^ (0xedb88320U & (0U - (crc & 1U)));
        }
    }
    return crc ^ 0xffffffffU;
}

std::uint32_t adler32(const std::vector<std::uint8_t>& bytes) {
    std::uint32_t a = 1;
    std::uint32_t b = 0;
    for (std::uint8_t byte : bytes) {
        a = (a + byte) % 65521U;
        b = (b + a) % 65521U;
    }
    return (b << 16) | a;
}

void append_u32_be(std::vector<std::uint8_t>& out, std::uint32_t value) {
    out.push_back(static_cast<std::uint8_t>((value >> 24) & 0xff));
    out.push_back(static_cast<std::uint8_t>((value >> 16) & 0xff));
    out.push_back(static_cast<std::uint8_t>((value >> 8) & 0xff));
    out.push_back(static_cast<std::uint8_t>(value & 0xff));
}

void append_chunk(std::vector<std::uint8_t>& png, const char* type, const std::vector<std::uint8_t>& data) {
    append_u32_be(png, static_cast<std::uint32_t>(data.size()));
    const std::size_t start = png.size();
    png.insert(png.end(), type, type + 4);
    png.insert(png.end(), data.begin(), data.end());
    std::vector<std::uint8_t> crc_data(png.begin() + static_cast<std::ptrdiff_t>(start), png.end());
    append_u32_be(png, crc32(crc_data));
}

struct Color {
    std::uint8_t r = 255;
    std::uint8_t g = 255;
    std::uint8_t b = 255;
};

struct Image {
    int width = 0;
    int height = 0;
    std::vector<Color> pixels;

    Image(int width, int height, Color color) : width(width), height(height), pixels(width * height, color) {}

    void set(int x, int y, Color color) {
        if (x < 0 || y < 0 || x >= width || y >= height) {
            return;
        }
        pixels[static_cast<std::size_t>(y * width + x)] = color;
    }

    Color get(int x, int y) const {
        return pixels[static_cast<std::size_t>(y * width + x)];
    }
};

std::vector<std::uint8_t> encode_png(const Image& image) {
    std::vector<std::uint8_t> raw;
    raw.reserve(static_cast<std::size_t>((image.width * 3 + 1) * image.height));
    for (int y = 0; y < image.height; ++y) {
        raw.push_back(0);
        for (int x = 0; x < image.width; ++x) {
            const Color pixel = image.get(x, y);
            raw.push_back(pixel.r);
            raw.push_back(pixel.g);
            raw.push_back(pixel.b);
        }
    }

    std::vector<std::uint8_t> zlib;
    zlib.push_back(0x78);
    zlib.push_back(0x01);
    std::size_t offset = 0;
    while (offset < raw.size()) {
        const std::size_t remaining = raw.size() - offset;
        const std::uint16_t block = static_cast<std::uint16_t>(std::min<std::size_t>(remaining, 65535));
        const bool final = offset + block == raw.size();
        zlib.push_back(final ? 1 : 0);
        zlib.push_back(static_cast<std::uint8_t>(block & 0xff));
        zlib.push_back(static_cast<std::uint8_t>((block >> 8) & 0xff));
        const std::uint16_t nlen = static_cast<std::uint16_t>(~block);
        zlib.push_back(static_cast<std::uint8_t>(nlen & 0xff));
        zlib.push_back(static_cast<std::uint8_t>((nlen >> 8) & 0xff));
        zlib.insert(zlib.end(), raw.begin() + static_cast<std::ptrdiff_t>(offset), raw.begin() + static_cast<std::ptrdiff_t>(offset + block));
        offset += block;
    }
    append_u32_be(zlib, adler32(raw));

    std::vector<std::uint8_t> png = {0x89, 'P', 'N', 'G', '\r', '\n', 0x1a, '\n'};
    std::vector<std::uint8_t> ihdr;
    append_u32_be(ihdr, static_cast<std::uint32_t>(image.width));
    append_u32_be(ihdr, static_cast<std::uint32_t>(image.height));
    ihdr.push_back(8);
    ihdr.push_back(2);
    ihdr.push_back(0);
    ihdr.push_back(0);
    ihdr.push_back(0);
    append_chunk(png, "IHDR", ihdr);
    append_chunk(png, "IDAT", zlib);
    append_chunk(png, "IEND", {});
    return png;
}
