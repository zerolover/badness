/* C ABI over a flat, immutable CST snapshot (mirrors src/ffi.rs; no cbindgen
 * in this repo, so keep both in sync by hand). Rowan never crosses this
 * boundary -- badness_parse_utf16 flattens it into BadnessCstNode, navigated
 * via `parent`/`first_child`/`next_sibling` indices, root at index 0.
 *
 * Every accessor below is a read-only bulk view (pointer + count) into an
 * opaque BadnessTree*, valid until that tree is freed. There is no per-item
 * getter; copy out what you need (see include/badness.hpp for a C++ type
 * that does this). */

#ifndef BADNESS_FFI_H
#define BADNESS_FFI_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define BADNESS_NONE UINT32_MAX /* "no such node" for parent/first_child/next_sibling */
#define BADNESS_NODE_TOKEN UINT16_C(1)  /* flags bit: a token, not a node */
#define BADNESS_NODE_TRIVIA UINT16_C(2) /* flags bit: whitespace/comment */

/* BADNESS_NULL_POINTER means the caller violated a documented precondition
 * (a caller-side bug); BADNESS_INVALID_INPUT covers anything about `source`
 * itself that keeps it from parsing (malformed UTF-16, or input too large
 * for a u32 offset). */
typedef enum BadnessStatus {
    BADNESS_OK,
    BADNESS_NULL_POINTER,
    BADNESS_INVALID_INPUT,
} BadnessStatus;

/* Opaque; release with badness_tree_free. */
typedef struct BadnessTree BadnessTree;

/* One CST node or token (see `flags`), in a flat preorder table, root at
 * index 0. `kind` mirrors the enum below (which mirrors Rust's SyntaxKind in
 * src/syntax.rs -- `build.rs` verifies their order). `start_utf16`/
 * `end_utf16` is a UTF-16 range, end exclusive. `parent`/`first_child`/
 * `next_sibling` are table indices or BADNESS_NONE; children form a
 * singly-linked list via first_child -> next_sibling in source order. */
typedef struct BadnessCstNode {
    uint16_t kind;
    uint16_t flags;
    uint32_t start_utf16;
    uint32_t end_utf16;
    uint32_t parent;
    uint32_t first_child;
    uint32_t next_sibling;
} BadnessCstNode;

/* A parser diagnostic, in the same UTF-16 ranges as BadnessCstNode.
 * `message` is NOT null-terminated; read exactly `message_len` bytes (UTF-8).
 * It is valid only until the owning BadnessTree is passed to badness_tree_free. */
typedef struct BadnessDiagnostic {
    uint32_t start_utf16;
    uint32_t end_utf16;
    const char *message;
    uint32_t message_len;
} BadnessDiagnostic;

/* CST kind values mirror SyntaxKind's #[repr(u16)] discriminants. `build.rs`
 * checks this list and badness_cst_kind_name against src/syntax.rs. */
enum {
    BADNESS_CONTROL_WORD,
    BADNESS_CONTROL_SYMBOL,
    BADNESS_L_BRACE,
    BADNESS_R_BRACE,
    BADNESS_L_BRACKET,
    BADNESS_R_BRACKET,
    BADNESS_DOLLAR,
    BADNESS_AMPERSAND,
    BADNESS_HASH,
    BADNESS_CARET,
    BADNESS_UNDERSCORE,
    BADNESS_TILDE,
    BADNESS_COMMENT,
    BADNESS_WHITESPACE,
    BADNESS_NEWLINE,
    BADNESS_WORD,
    BADNESS_VERB,
    BADNESS_VERBATIM_BODY,
    BADNESS_DOC_MARGIN,
    BADNESS_GUARD,
    BADNESS_ERROR,
    BADNESS_GROUP,
    BADNESS_OPTIONAL,
    BADNESS_ARGUMENT,
    BADNESS_COMMAND,
    BADNESS_ENVIRONMENT,
    BADNESS_BEGIN,
    BADNESS_END,
    BADNESS_NAME_GROUP,
    BADNESS_INLINE_MATH,
    BADNESS_DISPLAY_MATH,
    BADNESS_MATH,
    BADNESS_SCRIPTED,
    BADNESS_SUBSCRIPT,
    BADNESS_SUPERSCRIPT,
    BADNESS_LEFT_RIGHT,
    BADNESS_PARAGRAPH,
    BADNESS_DOC_COMMENT,
    BADNESS_TEXT,
    BADNESS_LINE_BREAK,
    BADNESS_ROOT,
};

/* Debug/display name for `kind`; "UNKNOWN" if out of range. */
static inline const char *badness_cst_kind_name(uint16_t kind) {
    switch (kind) {
    case BADNESS_CONTROL_WORD: return "CONTROL_WORD";
    case BADNESS_CONTROL_SYMBOL: return "CONTROL_SYMBOL";
    case BADNESS_L_BRACE: return "L_BRACE";
    case BADNESS_R_BRACE: return "R_BRACE";
    case BADNESS_L_BRACKET: return "L_BRACKET";
    case BADNESS_R_BRACKET: return "R_BRACKET";
    case BADNESS_DOLLAR: return "DOLLAR";
    case BADNESS_AMPERSAND: return "AMPERSAND";
    case BADNESS_HASH: return "HASH";
    case BADNESS_CARET: return "CARET";
    case BADNESS_UNDERSCORE: return "UNDERSCORE";
    case BADNESS_TILDE: return "TILDE";
    case BADNESS_COMMENT: return "COMMENT";
    case BADNESS_WHITESPACE: return "WHITESPACE";
    case BADNESS_NEWLINE: return "NEWLINE";
    case BADNESS_WORD: return "WORD";
    case BADNESS_VERB: return "VERB";
    case BADNESS_VERBATIM_BODY: return "VERBATIM_BODY";
    case BADNESS_DOC_MARGIN: return "DOC_MARGIN";
    case BADNESS_GUARD: return "GUARD";
    case BADNESS_ERROR: return "ERROR";
    case BADNESS_GROUP: return "GROUP";
    case BADNESS_OPTIONAL: return "OPTIONAL";
    case BADNESS_ARGUMENT: return "ARGUMENT";
    case BADNESS_COMMAND: return "COMMAND";
    case BADNESS_ENVIRONMENT: return "ENVIRONMENT";
    case BADNESS_BEGIN: return "BEGIN";
    case BADNESS_END: return "END";
    case BADNESS_NAME_GROUP: return "NAME_GROUP";
    case BADNESS_INLINE_MATH: return "INLINE_MATH";
    case BADNESS_DISPLAY_MATH: return "DISPLAY_MATH";
    case BADNESS_MATH: return "MATH";
    case BADNESS_SCRIPTED: return "SCRIPTED";
    case BADNESS_SUBSCRIPT: return "SUBSCRIPT";
    case BADNESS_SUPERSCRIPT: return "SUPERSCRIPT";
    case BADNESS_LEFT_RIGHT: return "LEFT_RIGHT";
    case BADNESS_PARAGRAPH: return "PARAGRAPH";
    case BADNESS_DOC_COMMENT: return "DOC_COMMENT";
    case BADNESS_TEXT: return "TEXT";
    case BADNESS_LINE_BREAK: return "LINE_BREAK";
    case BADNESS_ROOT: return "ROOT";
    default: return "UNKNOWN";
    }
}

/* Parses `source`/`source_len` (UTF-16 code units; caller retains ownership,
 * contents are copied). On success *out_tree is a new BadnessTree* (free
 * with badness_tree_free) and the return is BADNESS_OK; on failure *out_tree
 * is null and the return explains why. A null `source` is only valid when
 * `source_len` is 0. */
BadnessStatus badness_parse_utf16(const uint16_t *source, size_t source_len, BadnessTree **out_tree);

/* Bulk read-only views into `tree`: badness_tree_nodes returns every
 * BadnessCstNode in preorder, badness_tree_diagnostics every diagnostic.
 * Null `tree` yields null + *out_count = 0. Returned pointers (and, for
 * diagnostics, each `message`) are valid until badness_tree_free. */
const BadnessCstNode *badness_tree_nodes(const BadnessTree *tree, uint32_t *out_count);
const BadnessDiagnostic *badness_tree_diagnostics(const BadnessTree *tree, uint32_t *out_count);

/* Frees a tree from badness_parse_utf16; null is a no-op. */
void badness_tree_free(BadnessTree *tree);

#ifdef __cplusplus
}
#endif

#endif
