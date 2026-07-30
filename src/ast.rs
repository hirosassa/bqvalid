//! A backend-neutral syntax tree.
//!
//! Rules used to navigate `tree_sitter::Node`/`Tree` directly. That coupled the
//! whole rule set to tree-sitter's API — in particular to `parent()`,
//! `next_named_sibling()`, `child_by_field_name()` and node identity via
//! `id()`, none of which a top-down parser like ZetaSQL/googlesql exposes.
//!
//! [`Ast`] decouples the rules from any one parser by *materializing* the tree
//! into an arena we own. Because we build the arena ourselves, we can populate
//! parent links, node identity (the arena index) and per-node field names even
//! for a backend that only hands us children top-down. Today the arena is built
//! from tree-sitter (see [`Ast::from_tree_sitter`]); a googlesql backend can
//! populate the same shape later without touching a single rule.
//!
//! The arena is a *faithful mirror* of the source tree: every node (named and
//! anonymous) is kept, with its kind, byte range, 0-based start position, field
//! name under its parent, parent link and ordered children. [`NodeRef`] offers
//! the same navigation surface the rules already relied on, so migrating a rule
//! is a rename rather than a rewrite.

use std::ops::Range;

/// A node's start position in the source, 0-based on both axes.
///
/// Mirrors tree-sitter's `Point` (same `row`/`column` field names) so callers
/// that read `start_position().row` keep working unchanged. Diagnostics convert
/// to 1-based via [`crate::rules::helpers::one_based_start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub row: usize,
    pub column: usize,
}

/// One arena entry. Private: navigation goes through [`NodeRef`].
struct NodeData {
    kind: String,
    named: bool,
    /// The field this node occupies in its parent, if any (tree-sitter field
    /// names). `None` for anonymous positions and for backends without fields.
    field_name: Option<String>,
    byte_range: Range<usize>,
    start: Point,
    parent: Option<usize>,
    children: Vec<usize>,
}

/// A parsed syntax tree, owned as a flat arena of [`NodeData`].
///
/// Index `0` is not assumed to be the root; [`Ast::root`] returns the recorded
/// root. Rules never see the arena directly — they navigate through
/// [`NodeRef`], which is a cheap `(arena, index)` handle.
pub struct Ast {
    nodes: Vec<NodeData>,
    root: usize,
}

impl Ast {
    /// The root node of the tree.
    #[must_use]
    pub const fn root(&self) -> NodeRef<'_> {
        NodeRef {
            ast: self,
            idx: self.root,
        }
    }

    /// Every node in pre-order (parent before children, children in order),
    /// starting at the root. Replaces `traverse(tree.root_node().walk(),
    /// Order::Pre)`.
    #[must_use]
    pub fn pre_order(&self) -> Vec<NodeRef<'_>> {
        self.root().pre_order()
    }

    /// Build the arena from a tree-sitter tree, capturing every node with its
    /// kind, named flag, field name, byte range, start position, parent link
    /// and ordered children.
    #[must_use]
    pub fn from_tree_sitter(tree: &tree_sitter::Tree) -> Self {
        let mut nodes: Vec<NodeData> = Vec::new();
        let root = build(tree.root_node(), None, None, &mut nodes);
        Self { nodes, root }
    }

    fn get(&self, idx: usize) -> Option<&NodeData> {
        self.nodes.get(idx)
    }
}

/// Recursively add `node` (occupying `field` under `parent`) and its subtree to
/// `nodes`, returning the new node's arena index.
fn build(
    node: tree_sitter::Node<'_>,
    field: Option<String>,
    parent: Option<usize>,
    nodes: &mut Vec<NodeData>,
) -> usize {
    let idx = nodes.len();
    let pos = node.start_position();
    nodes.push(NodeData {
        kind: node.kind().to_string(),
        named: node.is_named(),
        field_name: field,
        byte_range: node.start_byte()..node.end_byte(),
        start: Point {
            row: pos.row,
            column: pos.column,
        },
        parent,
        children: Vec::new(),
    });

    let mut children = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let field_name = cursor.field_name().map(ToString::to_string);
            let child_idx = build(cursor.node(), field_name, Some(idx), nodes);
            children.push(child_idx);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    if let Some(data) = nodes.get_mut(idx) {
        data.children = children;
    }
    idx
}

/// A cheap, `Copy` handle to one node in an [`Ast`].
///
/// Exposes the same navigation the rules relied on under tree-sitter —
/// `kind`, `parent`, `children`/`named_children`, `child_by_field_name`,
/// `next_named_sibling`, position and text — so a rule migrates by swapping the
/// type, not its logic.
#[derive(Clone, Copy)]
pub struct NodeRef<'a> {
    ast: &'a Ast,
    idx: usize,
}

impl<'a> NodeRef<'a> {
    fn data(&self) -> Option<&'a NodeData> {
        self.ast.get(self.idx)
    }

    const fn at(&self, idx: usize) -> Self {
        Self { ast: self.ast, idx }
    }

    /// Stable identity of this node within its tree. Replaces `Node::id()`;
    /// two `NodeRef`s are the same node iff their ids are equal.
    #[must_use]
    pub const fn id(&self) -> usize {
        self.idx
    }

    /// The node's kind (grammar symbol name). Empty string only for a corrupt
    /// arena, which cannot occur for a `NodeRef` obtained from an `Ast`.
    #[must_use]
    pub fn kind(&self) -> &'a str {
        self.data().map_or("", |d| d.kind.as_str())
    }

    /// Whether this is a named node (as opposed to an anonymous token).
    #[must_use]
    pub fn is_named(&self) -> bool {
        self.data().is_some_and(|d| d.named)
    }

    /// The node's byte range in the source.
    #[must_use]
    pub fn byte_range(&self) -> Range<usize> {
        self.data().map_or(0..0, |d| d.byte_range.clone())
    }

    /// Start byte offset in the source.
    #[must_use]
    pub fn start_byte(&self) -> usize {
        self.data().map_or(0, |d| d.byte_range.start)
    }

    /// End byte offset in the source.
    #[must_use]
    pub fn end_byte(&self) -> usize {
        self.data().map_or(0, |d| d.byte_range.end)
    }

    /// The node's 0-based start position.
    #[must_use]
    pub fn start_position(&self) -> Point {
        self.data().map_or(Point { row: 0, column: 0 }, |d| d.start)
    }

    /// The field name this node occupies in its parent, if any.
    #[must_use]
    pub fn field_name(&self) -> Option<&'a str> {
        self.data().and_then(|d| d.field_name.as_deref())
    }

    /// The node's source text, or `None` if the byte range does not map to a
    /// valid `&str` slice of `sql` (e.g. a tree/source mismatch that cuts a
    /// multibyte character). Replaces `Node::utf8_text`.
    #[must_use]
    pub fn text<'s>(&self, sql: &'s str) -> Option<&'s str> {
        sql.get(self.byte_range())
    }

    /// This node's parent, or `None` at the root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        self.data().and_then(|d| d.parent).map(|p| self.at(p))
    }

    /// All children in order, including anonymous token nodes. Replaces
    /// `node.children(&mut node.walk())`.
    #[must_use]
    pub fn children(&self) -> Vec<Self> {
        self.data()
            .map(|d| d.children.iter().map(|&c| self.at(c)).collect())
            .unwrap_or_default()
    }

    /// Named children in order (anonymous tokens skipped). Replaces
    /// `node.named_children(&mut node.walk())`.
    #[must_use]
    pub fn named_children(&self) -> Vec<Self> {
        self.children().into_iter().filter(Self::is_named).collect()
    }

    /// The `i`-th child (all children), or `None` if out of range.
    #[must_use]
    pub fn child(&self, i: usize) -> Option<Self> {
        self.data()
            .and_then(|d| d.children.get(i))
            .map(|&c| self.at(c))
    }

    /// The `i`-th named child, or `None` if out of range.
    #[must_use]
    pub fn named_child(&self, i: usize) -> Option<Self> {
        self.named_children().into_iter().nth(i)
    }

    /// The child occupying the named field `name`, if any. Replaces
    /// `Node::child_by_field_name`.
    #[must_use]
    pub fn child_by_field_name(&self, name: &str) -> Option<Self> {
        self.children()
            .into_iter()
            .find(|c| c.field_name() == Some(name))
    }

    /// The next named sibling following this node, if any. Replaces
    /// `Node::next_named_sibling`.
    #[must_use]
    pub fn next_named_sibling(&self) -> Option<Self> {
        let parent = self.parent()?;
        let siblings = parent.children();
        let mut after = siblings.into_iter().skip_while(|s| s.id() != self.idx);
        after.next(); // drop self
        after.find(Self::is_named)
    }

    /// This node and its whole subtree in pre-order (self first, then each
    /// child's subtree in order). Replaces `traverse(node.walk(), Order::Pre)`.
    #[must_use]
    pub fn pre_order(&self) -> Vec<Self> {
        let mut out = Vec::new();
        let mut stack = vec![*self];
        while let Some(node) = stack.pop() {
            out.push(node);
            // Push children reversed so they pop in source order.
            let children = node.children();
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
        out
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test code"
)]
mod tests {
    use super::*;
    use crate::rules::helpers::parse_sql;

    #[test]
    fn root_and_kinds_mirror_the_source_tree() {
        // `select a from t` must materialize the expected top-level shape with
        // tree-sitter's kind names preserved (Phase 1 keeps the vocabulary).
        let ast = parse_sql("select a from t");
        let kinds: Vec<&str> = ast.pre_order().iter().map(NodeRef::kind).collect();
        assert!(kinds.contains(&"select"), "kinds were: {kinds:?}");
        assert!(kinds.contains(&"identifier"));
        assert!(kinds.contains(&"from_clause"));
    }

    #[test]
    fn pre_order_visits_parent_before_children_in_source_order() {
        let ast = parse_sql("select a, b from t");
        let order = ast.pre_order();
        // The select_list must appear before the identifiers it contains, and
        // `a` must appear before `b`.
        let pos = |pred: &dyn Fn(&NodeRef) -> bool| order.iter().position(pred);
        let list = pos(&|n| n.kind() == "select_list").unwrap();
        let a = order
            .iter()
            .position(|n| n.kind() == "identifier" && n.text("select a, b from t") == Some("a"))
            .unwrap();
        let b = order
            .iter()
            .position(|n| n.kind() == "identifier" && n.text("select a, b from t") == Some("b"))
            .unwrap();
        assert!(list < a && a < b, "list={list} a={a} b={b}");
    }

    #[test]
    fn parent_links_point_back_up() {
        let ast = parse_sql("select a from t");
        let ident = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "identifier")
            .unwrap();
        // Walking parents must eventually reach a node with no parent (the root).
        let mut cur = Some(ident);
        let mut steps = 0;
        while let Some(n) = cur {
            cur = n.parent();
            steps += 1;
            assert!(steps < 100, "parent chain must terminate");
        }
        assert!(steps > 1, "identifier must have ancestors");
    }

    #[test]
    fn children_include_anonymous_named_children_do_not() {
        // A comma-separated select list has anonymous comma tokens between the
        // named select_expression children; children() sees them, the named
        // variant does not.
        let ast = parse_sql("select a, b from t");
        let list = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "select_list")
            .unwrap();
        assert!(
            list.children().len() > list.named_children().len(),
            "anonymous tokens (commas) must be visible via children() only"
        );
        assert!(list.named_children().iter().all(NodeRef::is_named));
    }

    #[test]
    fn start_position_is_zero_based_and_byte_range_extracts_text() {
        // `col1` starts at row 0, col 7 (0-based) and its byte range slices back
        // to "col1".
        let sql = "SELECT col1 FROM t";
        let ast = parse_sql(sql);
        let col1 = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "identifier" && n.text(sql) == Some("col1"))
            .unwrap();
        assert_eq!(col1.start_position(), Point { row: 0, column: 7 });
        assert_eq!(col1.start_byte(), 7);
        assert_eq!(col1.end_byte(), 11);
    }

    #[test]
    fn text_returns_none_when_range_splits_a_multibyte_char() {
        // A node's byte range applied to a *different* source that cuts a
        // multibyte character must yield None, not a panic or garbage.
        let sql = "SELECT col1 FROM t";
        let ast = parse_sql(sql);
        let col1 = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "identifier" && n.text(sql) == Some("col1"))
            .unwrap();
        assert_eq!(col1.byte_range(), 7..11);
        // byte 10 is the first byte of the 3-byte 'あ', so 7..11 cuts it.
        assert_eq!(col1.text("abcdefghijあ"), None);
    }

    #[test]
    fn next_named_sibling_skips_anonymous_tokens() {
        let sql = "select a, b from t";
        let ast = parse_sql(sql);
        let a = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "select_expression" && n.text(sql) == Some("a"))
            .unwrap();
        let next = a.next_named_sibling().expect("a has a named sibling b");
        assert_eq!(next.text(sql), Some("b"));
    }

    #[test]
    fn id_uniquely_identifies_nodes() {
        let ast = parse_sql("select a from t");
        let nodes = ast.pre_order();
        let root = ast.root();
        assert_eq!(root.id(), ast.root().id(), "same node -> same id");
        // Every node in a pre-order walk has a distinct id.
        let mut ids: Vec<usize> = nodes.iter().map(NodeRef::id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "ids must be unique");
    }
}
