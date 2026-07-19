#ifndef BADNESS_HPP
#define BADNESS_HPP

// Header-only C++ wrapper over badness_ffi.h's flat CST snapshot. Consumers
// just #include this alongside badness_ffi.h; there is no separate library
// target to build or link.
//
// This header is deliberately independent of any particular UI framework.
// Source/CST text uses std::u16string; diagnostics and completion strings stay
// UTF-8 until the caller's integration boundary.
//
#include <cassert>
#include <cstdint>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "badness_ffi.h"

namespace badness {

using NodeId = uint32_t;
inline constexpr NodeId kNoNode = BADNESS_NONE;

// A CST node or token. `kind` mirrors the BADNESS_* values from the C ABI.
// It remains uint16_t because the ABI exposes raw kind values, not an enum type.
//
// Children are stored CSR-style in SyntaxTree::childrenPool_. Each Node records
// its (childrenBegin, childrenCount) range, keeping nodes compact and allocation-free.
struct Node {
    uint16_t kind = 0;
    uint16_t flags = 0; // BADNESS_NODE_TOKEN / BADNESS_NODE_TRIVIA
    uint32_t start = 0; // UTF-16 offset
    uint32_t end = 0;   // UTF-16 offset, exclusive

    NodeId parent = kNoNode;
    uint32_t indexInParent = 0;
    uint32_t childrenBegin = 0; // start index into SyntaxTree::childrenPool_
    uint32_t childrenCount = 0;

    bool isToken() const { return (flags & BADNESS_NODE_TOKEN) != 0; }
    bool isTrivia() const { return (flags & BADNESS_NODE_TRIVIA) != 0; }
};

// Rust parser diagnostic.
struct SyntaxError {
    uint32_t start = 0; // UTF-16 offset
    uint32_t end = 0;   // UTF-16 offset, exclusive
    std::string message; // UTF-8
};

// A copied completion candidate.
struct CompletionCandidate {
    std::string label;
    uint16_t kind = 0;
    std::optional<std::string> insertText;
    bool snippet = false;
};

// A copied completion result.
struct CompletionResult {
    std::vector<CompletionCandidate> candidates;
};

// Copies an FFI completion result. The caller may free raw immediately after.
inline CompletionResult BuildCompletionResult(const BadnessCompletion* raw) {
    uint32_t count = 0;
    const BadnessCandidate* rawCandidates = badness_completion_candidates(raw, &count);
    CompletionResult result;
    result.candidates.reserve(count);
    for (uint32_t i = 0; i < count; ++i) {
        const BadnessCandidate& candidate = rawCandidates[i];
        CompletionCandidate copy;
        copy.label = std::string(candidate.label, candidate.label_len);
        copy.kind = candidate.kind;
        if (candidate.insert_text != nullptr) {
            copy.insertText = std::string(candidate.insert_text, candidate.insert_text_len);
        }
        copy.snippet = candidate.snippet;
        result.candidates.push_back(std::move(copy));
    }
    return result;
}

// An immutable CST snapshot from badness_parse_utf16. The constructor copies
// the FFI node and diagnostic views into local CSR storage. The caller retains
// ownership of `raw` and may free it once construction returns.
//
// Precondition: `source` must be the exact UTF-16 code-unit sequence that
// produced `raw` via badness_parse_utf16. If it isn't, start/end offsets no
// longer correspond to `source`'s content: text() may slice the wrong range
// or throw std::out_of_range from std::u16string_view::substr.
//
// Exception safety: if this constructor throws (e.g. std::bad_alloc while
// growing nodes_/childrenPool_/errors_), it does not free `raw` -- ownership
// stays with the caller per the contract above. We accept that risk rather
// than requiring every call site to wrap `raw` in an RAII guard: this
// constructor is expected not to throw in practice.
//
// Each non-root node appears exactly once in childrenPool_.
class SyntaxTree {
public:
    // Copies an FFI snapshot into this tree.
    SyntaxTree(const BadnessTree *raw, std::u16string source);

    // Original UTF-16 source text.
    const std::u16string &source() const { return source_; }

    // Parser diagnostics copied from the FFI snapshot.
    const std::vector<SyntaxError> &errors() const { return errors_; }

    // Root node ID, or kNoNode for an empty snapshot.
    NodeId root() const { return root_; }

    // Total number of nodes and tokens.
    uint32_t nodeCount() const { return static_cast<uint32_t>(nodes_.size()); }

    // Per-element kind, UTF-16 range, and flags.
    uint16_t kind(NodeId id) const { return nodes_[id].kind; }
    uint32_t start(NodeId id) const { return nodes_[id].start; }
    uint32_t end(NodeId id) const { return nodes_[id].end; }
    bool isToken(NodeId id) const { return nodes_[id].isToken(); }
    bool isTrivia(NodeId id) const { return nodes_[id].isTrivia(); }

    // Original UTF-16 source slice for an element.
    std::u16string_view text(NodeId id) const {
        const Node &n = nodes_[id];
        return std::u16string_view(source_).substr(n.start, n.end - n.start);
    }

    // Direct children in source order.
    std::span<const NodeId> children(NodeId id) const {
        const Node &n = nodes_[id];
        return std::span<const NodeId>(childrenPool_).subspan(n.childrenBegin, n.childrenCount);
    }

    // Immediate parent, if any.
    std::optional<NodeId> parent(NodeId id) const {
        const NodeId parent = nodes_[id].parent;
        return parent == kNoNode ? std::nullopt : std::optional(parent);
    }

    // This element followed by each ancestor through the root.
    std::vector<NodeId> ancestors(NodeId id) const {
        std::vector<NodeId> result;
        for (NodeId current = id; current != kNoNode; current = nodes_[current].parent) {
            result.push_back(current);
        }
        return result;
    }

    // This element and its subtree in preorder.
    std::vector<NodeId> descendants(NodeId id) const {
        std::vector<NodeId> result;
        std::vector<NodeId> pending = {id};
        while (!pending.empty()) {
            const NodeId current = pending.back();
            pending.pop_back();
            result.push_back(current);

            const auto childNodes = children(current);
            for (auto child = childNodes.rbegin(); child != childNodes.rend(); ++child) {
                pending.push_back(*child);
            }
        }
        return result;
    }

    // Next element with the same parent, whether node or token.
    std::optional<NodeId> nextSiblingOrToken(NodeId id) const {
        const Node &node = nodes_[id];
        const auto parentId = parent(id);
        if (!parentId || node.indexInParent + 1 == nodes_[*parentId].childrenCount) {
            return std::nullopt;
        }
        return childrenPool_[nodes_[*parentId].childrenBegin + node.indexInParent + 1];
    }

    // Previous element with the same parent, whether node or token.
    std::optional<NodeId> prevSiblingOrToken(NodeId id) const {
        const Node &node = nodes_[id];
        if (node.parent == kNoNode || node.indexInParent == 0) {
            return std::nullopt;
        }
        return childrenPool_[nodes_[node.parent].childrenBegin + node.indexInParent - 1];
    }

    // Next token in document order; trivia is included.
    std::optional<NodeId> nextToken(NodeId tokenId) const {
        assert(isToken(tokenId) && "nextToken requires a token node");
        for (NodeId id = tokenId + 1; id < nodeCount(); ++id) {
            if (isToken(id)) {
                return id;
            }
        }
        return std::nullopt;
    }

    // Previous token in document order; trivia is included.
    std::optional<NodeId> prevToken(NodeId tokenId) const {
        assert(isToken(tokenId) && "prevToken requires a token node");
        for (NodeId id = tokenId; id > 0; --id) {
            if (isToken(id - 1)) {
                return id - 1;
            }
        }
        return std::nullopt;
    }

    // None is {nullopt, nullopt}; a single token appears in both fields;
    // distinct fields mean that offset falls between the two tokens.
    struct TokenAtOffset {
        std::optional<NodeId> left;
        std::optional<NodeId> right;
    };

    // Tokens covering or adjacent to a UTF-16 offset.
    TokenAtOffset tokenAt(uint32_t offsetUtf16) const {
        if (offsetUtf16 > source_.size()) {
            return {};
        }

        std::optional<NodeId> previous;
        for (NodeId id = 0; id < nodeCount(); ++id) {
            if (!isToken(id)) {
                continue;
            }

            const Node &token = nodes_[id];
            if (token.start < offsetUtf16 && offsetUtf16 < token.end) {
                return {id, id};
            }
            if (offsetUtf16 == token.start) {
                if (previous && nodes_[*previous].end == offsetUtf16) {
                    return {previous, id};
                }
                return {id, id};
            }
            if (offsetUtf16 == token.end) {
                previous = id;
            }
        }

        return previous ? TokenAtOffset{previous, previous} : TokenAtOffset{};
    }

private:
    std::u16string source_;
    std::vector<Node> nodes_;       // preorder, root is nodes_[0]
    std::vector<NodeId> childrenPool_;
    NodeId root_ = kNoNode;
    std::vector<SyntaxError> errors_;
};

inline SyntaxTree::SyntaxTree(const BadnessTree *raw, std::u16string source)
    : source_(std::move(source)) {
    uint32_t nodeCount = 0;
    const BadnessCstNode *rawNodes = badness_tree_nodes(raw, &nodeCount);

    nodes_.resize(nodeCount);
    for (uint32_t i = 0; i < nodeCount; ++i) {
        nodes_[i].kind = rawNodes[i].kind;
        nodes_[i].flags = rawNodes[i].flags;
        nodes_[i].start = rawNodes[i].start_utf16;
        nodes_[i].end = rawNodes[i].end_utf16;
        nodes_[i].parent = rawNodes[i].parent;
    }

    // Pass 1: count each node's direct children via their `parent` field,
    // then prefix-sum into per-node (childrenBegin, childrenCount) slices.
    std::vector<uint32_t> childCount(nodeCount, 0);
    for (uint32_t i = 0; i < nodeCount; ++i) {
        const uint32_t p = rawNodes[i].parent;
        if (p != BADNESS_NONE) {
            assert(p < nodeCount && "node parent is outside the FFI node table");
            ++childCount[p];
        }
    }
    uint32_t running = 0;
    for (uint32_t i = 0; i < nodeCount; ++i) {
        nodes_[i].childrenBegin = running;
        nodes_[i].childrenCount = childCount[i];
        running += childCount[i];
    }
    childrenPool_.resize(running);

    // Pass 2: walk each node's first_child/next_sibling chain (the wire
    // format's linked-list representation) to fill childrenPool_ and each
    // child's indexInParent, in document order.
    std::vector<uint32_t> cursor(nodeCount);
    for (uint32_t i = 0; i < nodeCount; ++i) {
        cursor[i] = nodes_[i].childrenBegin;
    }
    for (uint32_t p = 0; p < nodeCount; ++p) {
        uint32_t index = 0;
        uint32_t child = rawNodes[p].first_child;
        while (child != BADNESS_NONE) {
            assert(child < nodeCount && "child is outside the FFI node table");
            assert(rawNodes[child].parent == p &&
                   "child chain disagrees with the child's parent field");
            // parent (pass 1) and first_child/next_sibling (this pass) are
            // redundant encodings of the same tree; this catches them ever
            // disagreeing (corrupted FFI data, ABI drift) before it silently
            // overruns the slice pass 1 reserved for p in childrenPool_.
            assert(cursor[p] < nodes_[p].childrenBegin + nodes_[p].childrenCount &&
                   "first_child/next_sibling chain longer than parent-derived child count");
            childrenPool_[cursor[p]] = child;
            ++cursor[p];
            nodes_[child].indexInParent = index;
            ++index;
            child = rawNodes[child].next_sibling;
        }
        assert(cursor[p] == nodes_[p].childrenBegin + nodes_[p].childrenCount &&
               "child chain shorter than parent-derived child count");
    }

    root_ = nodeCount > 0 ? 0 : kNoNode;

    uint32_t diagCount = 0;
    const BadnessDiagnostic *rawDiags = badness_tree_diagnostics(raw, &diagCount);
    errors_.resize(diagCount);
    for (uint32_t i = 0; i < diagCount; ++i) {
        errors_[i].start = rawDiags[i].start_utf16;
        errors_[i].end = rawDiags[i].end_utf16;
        errors_[i].message = std::string(rawDiags[i].message, rawDiags[i].message_len);
    }
}

} // namespace badness

#endif
