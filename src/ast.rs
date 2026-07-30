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

use googlesql::{AstNode, Module};

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

    /// Build the arena from a googlesql (ZetaSQL/WASM) parse of a **single**
    /// statement, reusing the given `module` (creating one compiles the WASM,
    /// which is expensive, so callers keep one around).
    ///
    /// The neutral shape is populated the same way as [`Ast::from_tree_sitter`],
    /// but from a backend that gives us far less: ZetaSQL exposes only each
    /// node's kind, its children and an optional byte range — no parent links,
    /// no node identity, no field names and no row/column. This constructor
    /// therefore synthesizes the parent links and identity (the arena index),
    /// derives 0-based positions from the byte offsets, and leaves `field_name`
    /// empty. Nodes ZetaSQL leaves without a byte range (the top-level
    /// statement/query/select, which begin at byte 0) get a range spanning
    /// their children.
    ///
    /// # Errors
    /// Returns the googlesql [`Error`](googlesql::Error) if the WASM call fails
    /// or the SQL is not exactly one syntactically valid statement (there is no
    /// error recovery and no multi-statement support).
    ///
    /// Note: the kinds here are ZetaSQL class names (`ASTSelect`, …), not the
    /// tree-sitter names the current rules match on, so no rule runs against
    /// this arena yet — wiring it into the analysis path is a later phase.
    pub fn from_googlesql(module: &mut Module, sql: &str) -> Result<Self, googlesql::Error> {
        let statement = module.parse_statement(sql)?;
        let line_starts = line_starts(sql);
        let mut nodes: Vec<NodeData> = Vec::new();
        let root = build_googlesql(statement.root(), None, &line_starts, &mut nodes);
        Ok(Self { nodes, root })
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

/// Byte offset of the start of each line in `sql` (line 0 starts at 0). Used to
/// turn a byte offset into a 0-based (row, column) without rescanning the whole
/// source per node.
fn line_starts(sql: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, byte) in sql.bytes().enumerate() {
        if byte == b'\n' {
            starts.push(i.saturating_add(1));
        }
    }
    starts
}

/// The 0-based (row, column) of `byte`, where `column` is a byte offset within
/// the line (matching tree-sitter's `Point`). `starts` must be the ascending
/// line-start table from [`line_starts`].
fn point_at(starts: &[usize], byte: usize) -> Point {
    let row = match starts.binary_search(&byte) {
        // `byte` is exactly a line start: column 0 of that line.
        Ok(i) => i,
        // Otherwise it falls inside the preceding line.
        Err(i) => i.saturating_sub(1),
    };
    let column = byte.saturating_sub(starts.get(row).copied().unwrap_or(0));
    Point { row, column }
}

/// Recursively add the googlesql `node` (under `parent`) and its subtree to
/// `nodes`, returning the new node's arena index.
///
/// ZetaSQL nodes are all "named" (there are no anonymous token nodes) and carry
/// no field names, so `named` is always `true` and `field_name` always `None`.
/// A node without a byte range gets one spanning its children.
fn build_googlesql(
    node: &AstNode,
    parent: Option<usize>,
    starts: &[usize],
    nodes: &mut Vec<NodeData>,
) -> usize {
    let idx = nodes.len();
    let own_range = node.byte_range();
    let range = own_range.clone().unwrap_or(0..0);
    nodes.push(NodeData {
        kind: node.kind().to_string(),
        named: true,
        field_name: None,
        start: point_at(starts, range.start),
        byte_range: range,
        parent,
        children: Vec::new(),
    });

    let children: Vec<usize> = node
        .children()
        .iter()
        .map(|child| build_googlesql(child, Some(idx), starts, nodes))
        .collect();

    // A rangeless node (the top-level statement/query/select) spans its
    // children; compute that only after they are built.
    let derived = if own_range.is_none() {
        let mut lo = usize::MAX;
        let mut hi = 0usize;
        for &child in &children {
            if let Some(data) = nodes.get(child) {
                lo = lo.min(data.byte_range.start);
                hi = hi.max(data.byte_range.end);
            }
        }
        (lo != usize::MAX).then_some(lo..hi)
    } else {
        None
    };

    if let Some(data) = nodes.get_mut(idx) {
        data.children = children;
        if let Some(span) = derived {
            data.start = point_at(starts, span.start);
            data.byte_range = span;
        }
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

/// Tests for the (dormant) googlesql/ZetaSQL backend. These exercise
/// [`Ast::from_googlesql`] directly — no rule uses it yet, since ZetaSQL kind
/// names differ from tree-sitter's (rule migration is Phase 3).
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "test code"
)]
mod googlesql_tests {
    use super::*;
    use googlesql::Module;

    /// A fresh googlesql module. `Module::new()` compiles the ZetaSQL WASM
    /// (~1s), so each test pays this once; reuse the returned module across as
    /// many parses as the test needs.
    fn module() -> Module {
        Module::new().expect("load googlesql wasm module")
    }

    #[test]
    fn from_googlesql_uses_zetasql_kind_names() {
        // The googlesql arena speaks ZetaSQL's vocabulary (AST* class names),
        // which is exactly why rules can't run on it until Phase 3.
        let sql = "select a from t";
        let ast = Ast::from_googlesql(&mut module(), sql).expect("parse");
        let kinds: Vec<&str> = ast.pre_order().iter().map(NodeRef::kind).collect();
        assert!(kinds.contains(&"ASTSelect"), "kinds were: {kinds:?}");
        assert!(kinds.contains(&"ASTFromClause"), "kinds were: {kinds:?}");
        assert!(kinds.contains(&"ASTIdentifier"), "kinds were: {kinds:?}");
        // And it does NOT speak tree-sitter's vocabulary.
        assert!(!kinds.contains(&"select"), "kinds were: {kinds:?}");
    }

    #[test]
    fn byte_ranges_extract_the_right_source_text() {
        // Every node's byte range must slice back to its own source text, so
        // rules (later) can read text the same way they do under tree-sitter.
        let sql = "select a from t";
        let ast = Ast::from_googlesql(&mut module(), sql).expect("parse");
        let ident = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "ASTIdentifier")
            .expect("an identifier");
        assert_eq!(ident.text(sql), Some("a"));
        let from = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "ASTFromClause")
            .expect("a from clause");
        assert_eq!(from.text(sql), Some("from t"));
    }

    #[test]
    fn positions_are_computed_from_byte_offsets_across_lines() {
        // ZetaSQL reports byte offsets only; from_googlesql must derive 0-based
        // (row, column) itself. In this two-line query, the GROUP BY `col1`
        // starts at byte 28 = row 1, column 16.
        let sql = "SELECT col1\nFROM t GROUP BY col1";
        let ast = Ast::from_googlesql(&mut module(), sql).expect("parse");
        let group_col = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "ASTIdentifier" && n.start_byte() == 28)
            .expect("the GROUP BY col1 identifier at byte 28");
        assert_eq!(group_col.text(sql), Some("col1"));
        assert_eq!(group_col.start_position(), Point { row: 1, column: 16 });
    }

    #[test]
    fn rangeless_top_nodes_get_a_range_derived_from_their_children() {
        // ZetaSQL leaves the top-level statement/query/select nodes without a
        // byte range (they begin at byte 0). from_googlesql fills these in from
        // the span of their children so navigation and text stay usable.
        let sql = "select a from t";
        let ast = Ast::from_googlesql(&mut module(), sql).expect("parse");
        let root = ast.root();
        assert_eq!(root.kind(), "ASTQueryStatement");
        // Derived range covers the child span (`a`@7 .. `t`@15), not 0..0.
        assert_eq!(root.byte_range(), 7..15);
        assert_eq!(root.text(sql), Some("a from t"));
    }

    #[test]
    fn parent_links_and_ids_are_synthesized() {
        // googlesql exposes no parent links or node identity; the arena
        // synthesizes both, exactly as it does for tree-sitter.
        let sql = "select a from t";
        let ast = Ast::from_googlesql(&mut module(), sql).expect("parse");
        let ident = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "ASTIdentifier")
            .expect("an identifier");
        // Walk to the root; the chain must terminate at a parentless node.
        let mut cur = Some(ident);
        let mut steps = 0;
        while let Some(n) = cur {
            cur = n.parent();
            steps += 1;
            assert!(steps < 100, "parent chain must terminate");
        }
        assert!(steps > 1, "identifier must have ancestors");
        // Ids are unique across the whole tree.
        let mut ids: Vec<usize> = ast.pre_order().iter().map(NodeRef::id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "ids must be unique");
    }

    #[test]
    fn parse_errors_propagate() {
        // No error recovery: a syntax error surfaces as Err rather than a
        // partial tree (an accepted behavior change for the migration).
        let err = Ast::from_googlesql(&mut module(), "select from where");
        assert!(err.is_err(), "invalid SQL must not yield a tree");
    }

    #[test]
    fn multiple_statements_are_rejected() {
        // ZetaSQL's parse_statement accepts a single statement only; multi
        // statement handling is deferred to the backend flip (Phase 3).
        let err = Ast::from_googlesql(&mut module(), "select 1; select 2;");
        assert!(err.is_err(), "multiple statements must be rejected");
    }
}
