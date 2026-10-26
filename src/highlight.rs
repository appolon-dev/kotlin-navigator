//! Document highlight: the declaration under the cursor and its exact usages in the buffer.

use tree_sitter::Node;

use crate::cancel::Cancel;
use crate::docs::byte_offset;
use crate::index::Index;
use crate::model::Span;
use crate::parse::span_at;
use crate::resolve::{Certainty, Current, Found, target_at};
use crate::sources::Sources;
use crate::usages::{certainty_of, occurrences, subject_at};

/// A place in the buffer to mark, and whether it is written or only read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub range: Span,
    pub write: bool,
}

/// Highlights of what the identifier at `byte` declares or refers to, sorted by (line,
/// start), each place once. Only the current buffer is looked at; probable places are left out.
pub fn highlight(index: &Index, current: &Current, sources: &Sources, byte: usize) -> Vec<Highlight> {
    // A default flag is never raised, so the subject is always found, not cancelled.
    let Ok(Some(subject)) = subject_at(index, current, sources, byte, &Cancel::default()) else { return Vec::new() };
    let mut out: Vec<Highlight> = Vec::new();
    for member in &subject.family {
        let (path, range) = match member {
            Found::Decl((file, decl)) => (&file.path, decl.name_range),
            Found::Local(local) => (&local.path, local.range),
        };
        if path == &current.summary.path {
            out.push(Highlight { range, write: true });
        }
    }
    for at in occurrences(current.tree, current.text, &subject.name) {
        let Some(resolution) = target_at(index, current, sources, at) else { continue };
        if certainty_of(current.summary, &resolution, &subject) != Some(Certainty::Exact) {
            continue;
        }
        let range = span_at(current.text, resolution.reference.range.clone());
        out.push(Highlight { range, write: is_written(current, range) });
    }
    out.sort_by_key(|h| (h.range.line, h.range.start, !h.write));
    // Of two finds of one place the first, a write, stays.
    out.dedup_by(|later, kept| later.range == kept.range);
    out
}

/// Whether the name at `range` is the left side of an assignment: `x = …`, `x += …`,
/// or the member name in `a.x = …`.
fn is_written(current: &Current, range: Span) -> bool {
    let Some(node) = byte_offset(current.text, range.line, range.start)
        .and_then(|b| current.tree.root_node().descendant_for_byte_range(b, b))
    else {
        return false;
    };
    let on_left = |parent: Node, child: Node| {
        parent.kind() == "assignment" && parent.child_by_field_name("left").is_some_and(|l| l.id() == child.id())
    };
    let Some(parent) = node.parent() else { return false };
    if on_left(parent, node) {
        return true;
    }
    // The name of `a.x`, not its receiver.
    parent.kind() == "navigation_expression"
        && parent.child(0).is_some_and(|first| first.id() != node.id())
        && parent.parent().is_some_and(|p| on_left(p, parent))
}
