//! C ABI over a lossless, UTF-16-ranged CST snapshot. Mirrored by hand in
//! `include/badness_ffi.h` (no cbindgen) — keep both in sync. Rowan types
//! never cross the C boundary; callers get a flat node table navigated via
//! `parent`/`first_child`/`next_sibling` indices.

use std::ffi::c_char;
use std::ptr;

use rowan::{GreenNode, NodeOrToken};

use crate::completion::{CandidateKind, candidates, classify_context};
use crate::parser::parse;
use crate::semantic::{SemanticModel, scan_definitions};
use crate::syntax::{SyntaxElement, SyntaxKind, SyntaxNode};

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
    /// The green tree, not rowan's red `SyntaxNode`: red nodes use non-atomic
    /// `Cell`s, so sharing one across threads via this FFI's raw pointer would
    /// race. `GreenNode` is `Send + Sync`; [`badness_tree_complete`] rebuilds
    /// a red root per call via `SyntaxNode::new_root`.
    green: GreenNode,
    /// Original UTF-8 source, retained to convert completion offsets from UTF-16.
    text: String,
    /// Kept alive, never mutated after construction, purely so `diagnostics_ffi`'s
    /// pointers into its `message` strings stay valid.
    #[allow(dead_code)]
    diagnostics: Vec<Diagnostic>,
    /// Precomputed bulk view over `diagnostics`, for [`badness_tree_diagnostics`].
    diagnostics_ffi: Vec<BadnessDiagnostic>,
}

/// An opaque completion result. Release with [`badness_completion_free`].
pub struct BadnessCompletion {
    /// Kept alive, never mutated after construction, so `candidates_ffi` string
    /// pointers remain valid.
    #[allow(dead_code)]
    candidates: Vec<CompletionCandidate>,
    candidates_ffi: Vec<BadnessCandidate>,
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

/// A completion candidate. UTF-8 strings remain valid until the owning
/// [`BadnessCompletion`] is freed; `insert_text` is null when absent.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BadnessCandidate {
    pub label: *const c_char,
    pub label_len: u32,
    pub kind: u16,
    pub insert_text: *const c_char,
    pub insert_text_len: u32,
    pub snippet: bool,
}

/// Owns UTF-8 candidate strings for a [`BadnessCompletion`].
struct CompletionCandidate {
    label: String,
    kind: CandidateKind,
    insert_text: Option<String>,
    snippet: bool,
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

/// Completes at `offset_utf16`, clamped to the end of the source when needed.
/// On success, writes a new [`BadnessCompletion`] to `out_completion`.
///
/// # Safety
///
/// `tree` must be a live [`BadnessTree`] and `out_completion` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badness_tree_complete(
    tree: *const BadnessTree,
    offset_utf16: u32,
    out_completion: *mut *mut BadnessCompletion,
) -> BadnessStatus {
    if out_completion.is_null() {
        return BadnessStatus::NullPointer;
    }
    // SAFETY: checked non-null above; `out_completion` is writable per this fn's contract.
    unsafe { *out_completion = ptr::null_mut() };
    if tree.is_null() {
        return BadnessStatus::NullPointer;
    }

    // SAFETY: checked non-null above; the caller upholds the lifetime contract.
    let tree = unsafe { &*tree };
    let offset = utf16_to_byte_offset(&tree.text, offset_utf16);
    let root = SyntaxNode::new_root(tree.green.clone());
    let context = classify_context(&root, offset);
    let user_sigs = scan_definitions(&root);
    let model = SemanticModel::build(&root);
    let candidates = candidates(&context, &user_sigs, &model);
    let completion = completion_snapshot(candidates);

    // SAFETY: `out_completion` is writable for this call per the function contract.
    unsafe { *out_completion = Box::into_raw(Box::new(completion)) };
    BadnessStatus::Ok
}

/// Read-only view of completion candidates; null completion yields null + zero.
///
/// # Safety
///
/// `completion` must be null or a live [`BadnessCompletion`]; `out_count` must
/// be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badness_completion_candidates(
    completion: *const BadnessCompletion,
    out_count: *mut u32,
) -> *const BadnessCandidate {
    let Some(completion) = (unsafe { completion.as_ref() }) else {
        // SAFETY: required by this function's contract.
        unsafe { *out_count = 0 };
        return ptr::null();
    };
    // SAFETY: required by this function's contract.
    unsafe { *out_count = completion.candidates_ffi.len() as u32 };
    completion.candidates_ffi.as_ptr()
}

/// Releases a completion from [`badness_tree_complete`]; null is a no-op.
///
/// # Safety
///
/// `completion` must be null or an unfreed pointer from
/// [`badness_tree_complete`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn badness_completion_free(completion: *mut BadnessCompletion) {
    if !completion.is_null() {
        // SAFETY: upheld by this function's safety contract.
        unsafe { drop(Box::from_raw(completion)) };
    }
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
    let green = root.green().into_owned();
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
        green,
        text,
        diagnostics,
        diagnostics_ffi,
    })
}

fn completion_snapshot(
    candidates: Vec<crate::completion::CompletionCandidate>,
) -> BadnessCompletion {
    let candidates: Vec<_> = candidates
        .into_iter()
        .map(|candidate| CompletionCandidate {
            label: candidate.label,
            kind: candidate.kind,
            insert_text: candidate.insert_text,
            snippet: candidate.snippet,
        })
        .collect();
    let candidates_ffi = candidates
        .iter()
        .map(|candidate| {
            let (insert_text, insert_text_len) = candidate
                .insert_text
                .as_ref()
                .map_or((ptr::null(), 0), |text| {
                    (text.as_ptr().cast(), text.len() as u32)
                });
            BadnessCandidate {
                label: candidate.label.as_ptr().cast(),
                label_len: candidate.label.len() as u32,
                kind: candidate.kind as u16,
                insert_text,
                insert_text_len,
                snippet: candidate.snippet,
            }
        })
        .collect();

    BadnessCompletion {
        candidates,
        candidates_ffi,
    }
}

/// Converts an absolute UTF-16 offset into a valid UTF-8 byte boundary.
/// Values past the source clamp to the end; offsets in a surrogate pair clamp
/// to that scalar's start.
fn utf16_to_byte_offset(text: &str, offset_utf16: u32) -> usize {
    let target = offset_utf16 as usize;
    let mut utf16_offset = 0usize;

    for (byte_offset, ch) in text.char_indices() {
        if target <= utf16_offset {
            return byte_offset;
        }
        let next = utf16_offset + ch.len_utf16();
        if target < next {
            return byte_offset;
        }
        if target == next {
            return byte_offset + ch.len_utf8();
        }
        utf16_offset = next;
    }

    text.len()
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

    #[test]
    fn completion_returns_utf8_candidates_from_the_tree_snapshot() {
        let source = "\\sec";
        let source_utf16: Vec<u16> = source.encode_utf16().collect();
        let tree = parse_utf16_snapshot(&source_utf16).unwrap();
        let mut completion: *mut BadnessCompletion = ptr::null_mut();

        let status =
            unsafe { badness_tree_complete(&tree, source_utf16.len() as u32, &mut completion) };
        assert_eq!(status, BadnessStatus::Ok);
        assert!(!completion.is_null());

        let mut count = 0;
        let candidates = unsafe { badness_completion_candidates(completion, &mut count) };
        assert!(count > 0);
        let section = unsafe { std::slice::from_raw_parts(candidates, count as usize) }
            .iter()
            .find(|candidate| {
                candidate.kind == CandidateKind::Command as u16
                    && std::str::from_utf8(unsafe {
                        std::slice::from_raw_parts(
                            candidate.label.cast::<u8>(),
                            candidate.label_len as usize,
                        )
                    })
                    .is_ok_and(|label| label == "section")
            });
        assert!(section.is_some());

        unsafe { badness_completion_free(completion) };
    }

    #[test]
    fn utf16_offsets_clamp_to_valid_utf8_boundaries() {
        let text = "a😀z";
        assert_eq!(utf16_to_byte_offset(text, 0), 0);
        assert_eq!(utf16_to_byte_offset(text, 1), 1);
        assert_eq!(utf16_to_byte_offset(text, 2), 1);
        assert_eq!(utf16_to_byte_offset(text, 3), 5);
        assert_eq!(utf16_to_byte_offset(text, 99), text.len());
    }

    #[test]
    fn completion_rejects_null_inputs() {
        let status = unsafe { badness_tree_complete(ptr::null(), 0, ptr::null_mut()) };
        assert_eq!(status, BadnessStatus::NullPointer);

        let mut completion = ptr::dangling_mut::<BadnessCompletion>();
        let status = unsafe { badness_tree_complete(ptr::null(), 0, &mut completion) };
        assert_eq!(status, BadnessStatus::NullPointer);
        assert!(completion.is_null());
    }
}
