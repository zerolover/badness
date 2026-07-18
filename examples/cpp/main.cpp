#if defined(_MSC_VER)
#define _SILENCE_CXX17_CODECVT_HEADER_DEPRECATION_WARNING
#endif

#include <codecvt>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <locale>
#include <optional>
#include <string>
#include <string_view>

#include "CLI11.hpp"
#include "badness.hpp"

namespace {

enum class InputKind { Text, File, Stdin };

struct InputSource {
    InputKind kind;
    std::string value;
};

struct Arguments {
    InputSource input;
    bool lineMode = false;
    bool complete = false;
    std::optional<size_t> offset;
};

[[noreturn]] void fail(const char* message, int status) {
    std::fprintf(stderr, "%s\n", message);
    std::exit(status);
}

Arguments parse_args(int argc, char* argv[]) {
    std::string text;
    std::string file;
    bool lineMode = false;
    bool complete = false;
    size_t offset = 0;

    CLI::App app("Badness C++ FFI parser example", "badness_example");
    app.set_help_flag("-h,--help", "Show this help message and exit");
    auto* inputOption = app.add_option("-i,--input", file, "Read LaTeX code from file");
    auto* textOption = app.add_option("latex", text, "LaTeX code")->expected(0, 1);
    inputOption->excludes(textOption);
    app.add_flag("-l,--line", lineMode, "Parse each non-empty line separately");
    app.add_flag("--complete", complete, "Unavailable until the completion C ABI is implemented");
    auto* offsetOption = app.add_option("--offset", offset, "UTF-8 byte offset for --complete");

    try {
        app.parse(argc, argv);
    } catch (const CLI::ParseError& error) {
        std::exit(app.exit(error));
    }

    const InputSource input = inputOption->count() > 0
                                  ? InputSource{InputKind::File, std::move(file)}
                                  : textOption->count() > 0
                                        ? InputSource{InputKind::Text, std::move(text)}
                                        : InputSource{InputKind::Stdin, {}};
    return {input, lineMode, complete, offsetOption->count() > 0 ? std::optional(offset) : std::nullopt};
}

std::string read_input(const InputSource& input) {
    switch (input.kind) {
    case InputKind::Text:
        return input.value;
    case InputKind::File: {
        std::ifstream file(input.value, std::ios::binary);
        if (!file) {
            std::fprintf(stderr, "Failed to read file '%s'\n", input.value.c_str());
            std::exit(1);
        }
        return {std::istreambuf_iterator<char>(file), {}};
    }
    case InputKind::Stdin:
        return {std::istreambuf_iterator<char>(std::cin), {}};
    }
    std::abort();
}

#if defined(__clang__)
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
#elif defined(__GNUC__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wdeprecated-declarations"
#endif

std::u16string to_utf16(std::string_view text) {
    std::wstring_convert<std::codecvt_utf8_utf16<char16_t>, char16_t> converter;
    return converter.from_bytes(text.data(), text.data() + text.size());
}

std::string to_utf8(std::u16string_view text) {
    std::wstring_convert<std::codecvt_utf8_utf16<char16_t>, char16_t> converter;
    return converter.to_bytes(text.data(), text.data() + text.size());
}

#if defined(__clang__)
#pragma clang diagnostic pop
#elif defined(__GNUC__)
#pragma GCC diagnostic pop
#endif

void print_tree(const badness::SyntaxTree& tree, badness::NodeId id, unsigned depth) {
    const std::string text = to_utf8(tree.text(id));
    std::printf(
        "%*s%s@%u..%u \"%s\"\n",
        static_cast<int>(depth * 2),
        "",
        badness_cst_kind_name(tree.kind(id)),
        tree.start(id),
        tree.end(id),
        text.c_str());

    for (badness::NodeId child : tree.children(id)) {
        print_tree(tree, child, depth + 1);
    }
}

bool parse_and_print(std::string_view utf8) {
    std::u16string source;
    try {
        source = to_utf16(utf8);
    } catch (const std::range_error&) {
        std::fprintf(stderr, "Error: input is not valid UTF-8\n");
        return false;
    }

    BadnessTree* raw = nullptr;
    const BadnessStatus status = badness_parse_utf16(
        reinterpret_cast<const uint16_t*>(source.data()), source.size(), &raw);
    if (status != BADNESS_OK) {
        std::fprintf(stderr, "badness_parse_utf16 failed: %d\n", static_cast<int>(status));
        return false;
    }

    badness::SyntaxTree tree(raw, std::move(source));
    badness_tree_free(raw);
    print_tree(tree, tree.root(), 0);
    for (const auto& error : tree.errors()) {
        std::fprintf(stderr, "error @%u..%u: %s\n", error.start, error.end, error.message.c_str());
    }
    return tree.errors().empty();
}

std::string_view trim(std::string_view text) {
    constexpr std::string_view whitespace = " \t\n\r\f\v";
    const size_t start = text.find_first_not_of(whitespace);
    if (start == std::string_view::npos) {
        return {};
    }
    return text.substr(start, text.find_last_not_of(whitespace) - start + 1);
}

void parse_lines(std::string_view input) {
    size_t start = 0;
    size_t lineNumber = 1;
    while (start < input.size()) {
        const size_t end = input.find('\n', start);
        const std::string_view line = trim(input.substr(start, end - start));
        if (!line.empty()) {
            std::printf("Line %zu:\n", lineNumber);
            parse_and_print(line);
            std::puts("");
        }
        if (end == std::string_view::npos) {
            break;
        }
        start = end + 1;
        ++lineNumber;
    }
}

} // namespace

int main(int argc, char* argv[]) {
    static_assert(sizeof(char16_t) == sizeof(uint16_t));

    const Arguments args = parse_args(argc, argv);
    const std::string input = read_input(args.input);
    if (args.complete) {
        if (args.lineMode) {
            fail("Error: --complete cannot be combined with --line", 2);
        }
        fail("Error: --complete is unavailable until the completion C ABI is implemented", 2);
    }

    if (args.lineMode) {
        parse_lines(input);
        return 0;
    }
    return parse_and_print(input) ? 0 : 1;
}
