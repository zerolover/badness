//! C ABI over a lossless, UTF-16-ranged CST snapshot. Mirrored by hand in
//! `include/badness_ffi.h` (no cbindgen) — keep both in sync. Rowan stays
//! internal; callers get a flat node table navigated via `parent`/
//! `first_child`/`next_sibling` indices.

use std::ffi::c_char;
use std::ptr;

use rowan::NodeOrToken;

use crate::parser::parse;
use crate::syntax::{SyntaxElement, SyntaxKind};

/// Sentinel used when an FFI node has no parent, child, or sibling.
pub const BADNESS_NONE: u32 = u32::MAX;
/// [`BadnessCstNode::flags`] bit indicating a terminal token rather than a CST node.
pub const BADNESS_NODE_TOKEN: u16 = 1;
/// [`BadnessCstNode::flags`] bit indicating whitespace/comment trivia.
pub const BADNESS_NODE_TRIVIA: u16 = 2;

/// The result of an FFI operation. [`NullPointer`](Self::NullPointer) is a
/// caller-side precondition violation; [`InvalidInput`](Self::InvalidInput)
/// covers anything wrong with `source` itself (malformed UTF-16, too large).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadnessStatus {
    Ok,
    NullPointer,
    InvalidInput,
}

/// An opaque owner of the CST snapshot. Release with [`badness_tree_free`],
/// never a C/C++ allocator.
pub struct BadnessTree {
    nodes: Vec<BadnessCstNode>,
    /// Kept alive, never mutated after construction, purely so `diagnostics_ffi`'s
    /// pointers into its `message` strings stay valid.
    #[allow(dead_code)]
    diagnostics: Vec<Diagnostic>,
    /// Precomputed bulk view over `diagnostics`, for [`badness_tree_diagnostics`].
    diagnostics_ffi: Vec<BadnessDiagnostic>,
}

/// A single CST element (node or token). `kind` mirrors a [`SyntaxKind`]
/// `#[repr(u16)]` discriminant; the range is UTF-16, end exclusive; the
/// relation fields are node indices or [`BADNESS_NONE`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadnessCstNode {
    pub kind: u16,
    pub flags: u16,
    pub start_utf16: u32,
    pub end_utf16: u32,
    pub parent: u32,
    pub first_child: u32,
    pub next_sibling: u32,
}

/// A parser diagnostic, in the same UTF-16 ranges as [`BadnessCstNode`].
/// `message` is **not** null-terminated (`message_len` UTF-8 bytes), and
/// valid only until the owning [`BadnessTree`] is freed.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BadnessDiagnostic {
    pub start_utf16: u32,
    pub end_utf16: u32,
    pub message: *const c_char,
    pub message_len: u32,
}

/// Owns a diagnostic's message text; unlike [`BadnessDiagnostic`] this never
/// crosses the FFI boundary (`String` isn't `#[repr(C)]`-able).
struct Diagnostic {
    start_utf16: u32,
    end_utf16: u32,
    message: String,
}

/// Parses `source` (UTF-16) into a CST; `source`'s contents are copied, so
/// the caller retains ownership. Null `*out_tree` on failure.
///
/// # Safety
///
/// `source` must address `source_len` valid `u16`s (or be null with
/// `source_len == 0`); `out_tree` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badness_parse_utf16(
    source: *const u16,
    source_len: usize,
    out_tree: *mut *mut BadnessTree,
) -> BadnessStatus {
    if out_tree.is_null() || (source.is_null() && source_len != 0) {
        return BadnessStatus::NullPointer;
    }

    // SAFETY: checked non-null above; `out_tree` is writable per this fn's contract.
    unsafe { *out_tree = ptr::null_mut() };

    let units: &[u16] = if source_len == 0 {
        &[]
    } else {
        // SAFETY: checked non-null above; caller guarantees this range.
        unsafe { std::slice::from_raw_parts(source, source_len) }
    };

    match parse_utf16_snapshot(units) {
        Ok(tree) => {
            // SAFETY: `out_tree` was checked above and remains valid for this call.
            unsafe { *out_tree = Box::into_raw(Box::new(tree)) };
            BadnessStatus::Ok
        }
        Err(status) => status,
    }
}

/// Read-only view of `tree`'s nodes (root at index 0); null for a null tree.
///
/// # Safety
///
/// `tree` must be null or a live [`BadnessTree`]; `out_count` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badness_tree_nodes(
    tree: *const BadnessTree,
    out_count: *mut u32,
) -> *const BadnessCstNode {
    let Some(tree) = (unsafe { tree.as_ref() }) else {
        unsafe { *out_count = 0 };
        return ptr::null();
    };
    unsafe { *out_count = tree.nodes.len() as u32 };
    tree.nodes.as_ptr()
}

/// Read-only view of `tree`'s parser diagnostics.
///
/// # Safety
///
/// Same as [`badness_tree_nodes`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badness_tree_diagnostics(
    tree: *const BadnessTree,
    out_count: *mut u32,
) -> *const BadnessDiagnostic {
    let Some(tree) = (unsafe { tree.as_ref() }) else {
        unsafe { *out_count = 0 };
        return ptr::null();
    };
    unsafe { *out_count = tree.diagnostics_ffi.len() as u32 };
    tree.diagnostics_ffi.as_ptr()
}

/// Releases a tree from [`badness_parse_utf16`]; null is a no-op.
///
/// # Safety
///
/// `tree` must be null or an unfreed pointer from [`badness_parse_utf16`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badness_tree_free(tree: *mut BadnessTree) {
    if !tree.is_null() {
        // SAFETY: upheld by this function's safety contract.
        unsafe { drop(Box::from_raw(tree)) };
    }
}

/// Decodes, parses, and flattens `units` into a [`BadnessTree`].
fn parse_utf16_snapshot(units: &[u16]) -> Result<BadnessTree, BadnessStatus> {
    if units.len() > u32::MAX as usize {
        return Err(BadnessStatus::InvalidInput);
    }
    let text = String::from_utf16(units).map_err(|_| BadnessStatus::InvalidInput)?;
    if text.len() > u32::MAX as usize {
        return Err(BadnessStatus::InvalidInput);
    }

    let parsed = parse(&text);
    let root = parsed.syntax();
    let utf16_offsets = byte_to_utf16_offsets(&text)?;
    let mut nodes = Vec::new();
    let root_id = lower_element(
        NodeOrToken::Node(root),
        BADNESS_NONE,
        &utf16_offsets,
        &mut nodes,
    );
    debug_assert_eq!(root_id, 0);

    let diagnostics: Vec<Diagnostic> = parsed
        .errors
        .into_iter()
        .map(|err| Diagnostic {
            start_utf16: utf16_offsets[err.start],
            end_utf16: utf16_offsets[err.end],
            message: err.message,
        })
        .collect();

    let diagnostics_ffi = diagnostics
        .iter()
        .map(|d| BadnessDiagnostic {
            start_utf16: d.start_utf16,
            end_utf16: d.end_utf16,
            message: d.message.as_ptr().cast(),
            message_len: d.message.len() as u32,
        })
        .collect();

    Ok(BadnessTree {
        nodes,
        diagnostics,
        diagnostics_ffi,
    })
}

/// Map each UTF-8 character boundary in `text` to its UTF-16 code-unit offset.
fn byte_to_utf16_offsets(text: &str) -> Result<Vec<u32>, BadnessStatus> {
    let mut offsets = vec![0; text.len() + 1];
    let mut utf16_offset = 0u32;

    for (byte_offset, ch) in text.char_indices() {
        offsets[byte_offset] = utf16_offset;
        utf16_offset = utf16_offset
            .checked_add(ch.len_utf16() as u32)
            .ok_or(BadnessStatus::InvalidInput)?;
        offsets[byte_offset + ch.len_utf8()] = utf16_offset;
    }

    Ok(offsets)
}

/// Recursively pushes `element` and its subtree onto `out`, rowan's red tree
/// -> flat [`BadnessCstNode`] table. Returns `element`'s new index.
fn lower_element(
    element: SyntaxElement,
    parent: u32,
    utf16_offsets: &[u32],
    out: &mut Vec<BadnessCstNode>,
) -> u32 {
    let kind = element.kind();
    let range = element.text_range();
    let id = out.len() as u32;
    let is_token = matches!(element, NodeOrToken::Token(_));
    let flags = if is_token {
        BADNESS_NODE_TOKEN | trivia_flag(kind)
    } else {
        0
    };

    out.push(BadnessCstNode {
        kind: kind as u16,
        flags,
        start_utf16: utf16_offsets[usize::from(range.start())],
        end_utf16: utf16_offsets[usize::from(range.end())],
        parent,
        first_child: BADNESS_NONE,
        next_sibling: BADNESS_NONE,
    });

    if let NodeOrToken::Node(node) = element {
        let mut previous = BADNESS_NONE;
        for child in node.children_with_tokens() {
            let child_id = lower_element(child, id, utf16_offsets, out);
            if previous == BADNESS_NONE {
                out[id as usize].first_child = child_id;
            } else {
                out[previous as usize].next_sibling = child_id;
            }
            previous = child_id;
        }
    }

    id
}

/// [`BADNESS_NODE_TRIVIA`] if `kind` is whitespace/comment trivia, else 0.
/// Mirrors the parser's private `Parser::is_trivia` (`src/parser/grammar.rs`,
/// also re-derived in `src/semantic/define.rs`) — keep this list in sync with
/// that one, not the other way around.
fn trivia_flag(kind: SyntaxKind) -> u16 {
    if matches!(
        kind,
        SyntaxKind::WHITESPACE
            | SyntaxKind::NEWLINE
            | SyntaxKind::COMMENT
            | SyntaxKind::DOC_MARGIN
            | SyntaxKind::GUARD
    ) {
        BADNESS_NODE_TRIVIA
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_uses_utf16_ranges_and_linked_children() {
        let source = "a😀\\frac{1}";
        let source_utf16: Vec<u16> = source.encode_utf16().collect();
        let tree = parse_utf16_snapshot(&source_utf16).unwrap();

        let command = tree
            .nodes
            .iter()
            .find(|node| node.kind == SyntaxKind::CONTROL_WORD as u16)
            .unwrap();
        assert_eq!((command.start_utf16, command.end_utf16), (3, 8));
        assert_eq!(
            String::from_utf16(
                &source_utf16[command.start_utf16 as usize..command.end_utf16 as usize]
            )
            .unwrap(),
            "\\frac"
        );

        for (id, node) in tree.nodes.iter().enumerate() {
            let mut child = node.first_child;
            while child != BADNESS_NONE {
                assert_eq!(tree.nodes[child as usize].parent, id as u32);
                child = tree.nodes[child as usize].next_sibling;
            }
        }
    }

    #[test]
    fn unclosed_group_reports_a_utf16_ranged_diagnostic() {
        let source = "a😀{unclosed";
        let source_utf16: Vec<u16> = source.encode_utf16().collect();
        let tree = parse_utf16_snapshot(&source_utf16).unwrap();

        assert_eq!(tree.diagnostics.len(), 1);
        let diag = &tree.diagnostics[0];
        // The `{` sits after "a😀" (1 + 2 UTF-16 units in), a single unit wide.
        assert_eq!((diag.start_utf16, diag.end_utf16), (3, 4));
        assert!(diag.message.contains("unclosed"), "{}", diag.message);

        let tree_ptr: *mut BadnessTree = Box::into_raw(Box::new(tree));
        let mut count: u32 = 0;
        let diags = unsafe { badness_tree_diagnostics(tree_ptr, &mut count) };
        assert_eq!(count, 1);
        let ffi_diag = unsafe { *diags };
        assert!(!ffi_diag.message.is_null());
        let message = unsafe {
            std::slice::from_raw_parts(ffi_diag.message.cast::<u8>(), ffi_diag.message_len as usize)
        };
        assert_eq!(std::str::from_utf8(message).unwrap(), "unclosed `{`");
        unsafe { badness_tree_free(tree_ptr) };
    }

    #[test]
    fn invalid_utf16_is_rejected_without_a_tree() {
        assert!(matches!(
            parse_utf16_snapshot(&[0xD800]),
            Err(BadnessStatus::InvalidInput)
        ));
    }

    #[test]
    fn badness_parse_utf16_rejects_a_null_out_tree() {
        let source: Vec<u16> = "x".encode_utf16().collect();
        let status = unsafe { badness_parse_utf16(source.as_ptr(), source.len(), ptr::null_mut()) };
        assert_eq!(status, BadnessStatus::NullPointer);
    }

    #[test]
    fn badness_parse_utf16_rejects_a_null_source_with_nonzero_len() {
        let mut tree_ptr: *mut BadnessTree = ptr::null_mut();
        let status = unsafe { badness_parse_utf16(ptr::null(), 1, &mut tree_ptr) };
        assert_eq!(status, BadnessStatus::NullPointer);
        assert!(tree_ptr.is_null());
    }

    #[test]
    fn badness_parse_utf16_accepts_a_null_source_when_len_is_zero() {
        let mut tree_ptr: *mut BadnessTree = ptr::null_mut();
        let status = unsafe { badness_parse_utf16(ptr::null(), 0, &mut tree_ptr) };
        assert_eq!(status, BadnessStatus::Ok);
        assert!(!tree_ptr.is_null());
        unsafe { badness_tree_free(tree_ptr) };
    }

    #[test]
    fn badness_parse_utf16_round_trips_through_the_real_entry_point() {
        let source: Vec<u16> = "\\frac{1}{2}".encode_utf16().collect();
        let mut tree_ptr: *mut BadnessTree = ptr::null_mut();
        let status = unsafe { badness_parse_utf16(source.as_ptr(), source.len(), &mut tree_ptr) };
        assert_eq!(status, BadnessStatus::Ok);
        assert!(!tree_ptr.is_null());

        let mut count: u32 = 0;
        let nodes = unsafe { badness_tree_nodes(tree_ptr, &mut count) };
        assert!(!nodes.is_null());
        assert!(count > 0);
        let root = unsafe { *nodes };
        assert_eq!(root.kind, SyntaxKind::ROOT as u16);

        unsafe { badness_tree_free(tree_ptr) };
    }

    #[test]
    fn bulk_views_are_null_with_zero_count_for_a_null_tree() {
        let mut count: u32 = 1; // deliberately nonzero, to prove it gets reset
        let nodes = unsafe { badness_tree_nodes(ptr::null(), &mut count) };
        assert!(nodes.is_null());
        assert_eq!(count, 0);

        count = 1;
        let diags = unsafe { badness_tree_diagnostics(ptr::null(), &mut count) };
        assert!(diags.is_null());
        assert_eq!(count, 0);
    }

    #[test]
    fn badness_tree_free_of_null_is_a_no_op() {
        unsafe { badness_tree_free(ptr::null_mut()) };
    }
}
