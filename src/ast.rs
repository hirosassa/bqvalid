//! A backend-neutral syntax tree.
//!
//! Rules navigate through this owned arena rather than a parser's own node type,
//! so they depend on `parent()`, `next_named_sibling()` and node identity via
//! `id()` — none of which a top-down parser like ZetaSQL/googlesql exposes
//! directly.
//!
//! [`Ast`] decouples the rules from the parser by *materializing* the tree into
//! an arena we own. Because we build the arena ourselves, we can synthesize
//! parent links and node identity (the arena index) even for a backend that only
//! hands us children top-down. The arena is built from googlesql (ZetaSQL); see
//! [`Ast::from_googlesql`].
//!
//! The arena is a *faithful mirror* of the source tree: every node is kept, with
//! its kind, byte range, 0-based start position, parent link and ordered
//! children. [`NodeRef`] offers a stable navigation surface for the rules.

use std::ops::Range;

use googlesql::{AstNode, Module};

/// A node's start position in the source, 0-based on both axes. `column`
/// counts chars (Unicode scalar values) from the line start, not bytes.
///
/// The `row`/`column` field names let callers read `start_position().row`
/// directly. Diagnostics convert to 1-based via
/// [`crate::rules::helpers::one_based_start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub row: usize,
    pub column: usize,
}

/// One arena entry. Private: navigation goes through [`NodeRef`].
struct NodeData {
    kind: String,
    named: bool,
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

    /// Build the arena from a googlesql (ZetaSQL/WASM) parse of a **single**
    /// statement, reusing the given `module` (creating one compiles the WASM,
    /// which is expensive, so callers keep one around).
    ///
    /// ZetaSQL exposes only each node's kind, its children and an optional byte
    /// range — no parent links, no node identity and no row/column. This
    /// constructor therefore synthesizes the parent links and identity (the
    /// arena index) and derives 0-based positions from the byte offsets. Nodes
    /// ZetaSQL reports without a byte range (in practice the top-level
    /// statement/query/select) get a range spanning their located children.
    ///
    /// # Errors
    /// Returns the googlesql [`Error`](googlesql::Error) if the WASM call fails
    /// or the SQL is not exactly one syntactically valid statement (there is no
    /// error recovery and no multi-statement support).
    ///
    /// The kinds here are ZetaSQL class names (`ASTSelect`, …), which is the
    /// vocabulary every rule matches on.
    pub fn from_googlesql(module: &mut Module, sql: &str) -> Result<Self, googlesql::Error> {
        let statement = module.parse_statement(sql)?;
        Ok(Self::from_googlesql_root(statement.root(), sql))
    }

    /// Build the arena from an already-parsed ZetaSQL statement `root`.
    ///
    /// The node byte offsets must be relative to the whole `sql`, which is how
    /// `Module::parse_statements` reports them for every statement in a
    /// semicolon-separated script (not just the first). Driving multi-statement
    /// analysis therefore means calling this once per statement with the shared
    /// script as `sql`, so row/column stay correct past the first statement.
    pub fn from_googlesql_root(root: &AstNode, sql: &str) -> Self {
        let index = LineIndex::new(sql);
        let mut nodes: Vec<NodeData> = Vec::new();
        let root = build_googlesql(root, None, &index, &mut nodes);
        Self { nodes, root }
    }

    fn get(&self, idx: usize) -> Option<&NodeData> {
        self.nodes.get(idx)
    }
}

/// Line-start table over `sql`, used to turn a byte offset into a 0-based
/// (row, column) without rescanning the whole source per node.
struct LineIndex<'a> {
    sql: &'a str,
    /// Byte offset of the start of each line (line 0 starts at 0).
    starts: Vec<usize>,
    /// When every byte is ASCII, char column equals byte column, which lets
    /// `point_at` skip rescanning the line (long minified lines stay O(log n)).
    ascii: bool,
}

impl<'a> LineIndex<'a> {
    fn new(sql: &'a str) -> Self {
        let mut starts = vec![0usize];
        for (i, byte) in sql.bytes().enumerate() {
            if byte == b'\n' {
                starts.push(i.saturating_add(1));
            }
        }
        Self {
            sql,
            starts,
            ascii: sql.is_ascii(),
        }
    }

    /// The 0-based (row, column) of `byte`, where `column` is the number of
    /// chars (Unicode scalar values) between the line start and `byte`.
    ///
    /// A `byte` inside a multibyte char maps to the column of that char, and a
    /// `byte` past the end is clamped to the end of `sql`; neither panics.
    fn point_at(&self, byte: usize) -> Point {
        // Snapping back to a char boundary (and to `len`) makes the slice below
        // valid and maps a mid-char `byte` to the char it falls inside.
        let byte = self.sql.floor_char_boundary(byte);
        let row = match self.starts.binary_search(&byte) {
            // `byte` is exactly a line start: column 0 of that line.
            Ok(i) => i,
            // Otherwise it falls inside the preceding line.
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.starts.get(row).copied().unwrap_or(0);
        let column = if self.ascii {
            byte.saturating_sub(line_start)
        } else {
            self.sql
                .get(line_start..byte)
                .map_or(0, |line| line.chars().count())
        };
        Point { row, column }
    }
}

/// The span covering the child ranges that carry a real location, or `None` if
/// none do.
///
/// A child left with the degenerate empty range `0..0` is a subtree ZetaSQL
/// reported no byte range for (and whose descendants had none either); it
/// carries no usable location, so it is skipped rather than pulling the span's
/// start down to byte 0. Since statements begin with a keyword (which ZetaSQL
/// emits no node for), a genuine range never starts at byte 0, so an exact
/// `0..0` unambiguously means "locationless".
fn derive_span<'a>(children: impl Iterator<Item = &'a Range<usize>>) -> Option<Range<usize>> {
    let mut lo = usize::MAX;
    let mut hi = 0usize;
    for range in children {
        if range.start == 0 && range.end == 0 {
            continue;
        }
        lo = lo.min(range.start);
        hi = hi.max(range.end);
    }
    (lo != usize::MAX).then_some(lo..hi)
}

/// Recursively add the googlesql `node` (under `parent`) and its subtree to
/// `nodes`, returning the new node's arena index.
///
/// ZetaSQL nodes are all "named" (there are no anonymous token nodes), so
/// `named` is always `true` and `named_children()` equals `children()`.
///
/// A node ZetaSQL reports no byte range for gets one spanning its located
/// children (see [`derive_span`]).
fn build_googlesql(
    node: &AstNode,
    parent: Option<usize>,
    index: &LineIndex<'_>,
    nodes: &mut Vec<NodeData>,
) -> usize {
    let idx = nodes.len();
    let own_range = node.byte_range();
    let range = own_range.clone().unwrap_or(0..0);
    nodes.push(NodeData {
        kind: node.kind().to_string(),
        named: true,
        start: index.point_at(range.start),
        byte_range: range,
        parent,
        children: Vec::new(),
    });

    let children: Vec<usize> = node
        .children()
        .iter()
        .map(|child| build_googlesql(child, Some(idx), index, nodes))
        .collect();

    // A rangeless node (the top-level statement/query/select) spans its located
    // children; compute that only after they are built.
    let derived = if own_range.is_none() {
        derive_span(
            children
                .iter()
                .filter_map(|&child| nodes.get(child).map(|data| &data.byte_range)),
        )
    } else {
        None
    };

    if let Some(data) = nodes.get_mut(idx) {
        data.children = children;
        if let Some(span) = derived {
            data.start = index.point_at(span.start);
            data.byte_range = span;
        }
    }
    idx
}

/// A cheap, `Copy` handle to one node in an [`Ast`].
///
/// Exposes the navigation the rules rely on — `kind`, `parent`,
/// `children`/`named_children`, `next_named_sibling`, position and text.
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
    fn pre_order_visits_parent_before_children_in_source_order() {
        let sql = "select a, b from t";
        let ast = parse_sql(sql);
        let order = ast.pre_order();
        // The select_list must appear before the column references it contains,
        // and `a` must appear before `b`.
        let pos = |pred: &dyn Fn(&NodeRef) -> bool| order.iter().position(pred);
        let list = pos(&|n| n.kind() == "ASTSelectList").unwrap();
        let a = order
            .iter()
            .position(|n| n.kind() == "ASTPathExpression" && n.text(sql) == Some("a"))
            .unwrap();
        let b = order
            .iter()
            .position(|n| n.kind() == "ASTPathExpression" && n.text(sql) == Some("b"))
            .unwrap();
        assert!(list < a && a < b, "list={list} a={a} b={b}");
    }

    #[test]
    fn every_node_is_named_on_googlesql() {
        // ZetaSQL emits no anonymous token nodes, so children() and
        // named_children() coincide for every node (there are no anonymous
        // tokens such as the commas in a select list).
        let ast = parse_sql("select a, b from t");
        for node in ast.pre_order() {
            assert!(node.is_named(), "kind {} must be named", node.kind());
            assert_eq!(
                node.children().len(),
                node.named_children().len(),
                "kind {} must have no anonymous children",
                node.kind()
            );
        }
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
            .find(|n| n.kind() == "ASTPathExpression" && n.text(sql) == Some("col1"))
            .unwrap();
        assert_eq!(col1.start_position(), Point { row: 0, column: 7 });
        assert_eq!(col1.start_byte(), 7);
        assert_eq!(col1.end_byte(), 11);
    }

    /// 0-based (row, column) of the first `ASTPathExpression` whose text is `name`.
    fn position_of(sql: &str, name: &str) -> Point {
        parse_sql(sql)
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "ASTPathExpression" && n.text(sql) == Some(name))
            .unwrap()
            .start_position()
    }

    #[test]
    fn column_counts_chars_not_bytes_after_japanese_text() {
        // "日本語" is 9 bytes but 3 chars, so `x` sits at char column 14, not 20.
        let point = position_of("SELECT '日本語', x FROM t", "x");
        assert_eq!(point, Point { row: 0, column: 14 });
    }

    #[test]
    fn column_counts_a_four_byte_emoji_as_one_char() {
        let point = position_of("SELECT '😀', x FROM t", "x");
        assert_eq!(point, Point { row: 0, column: 12 });
    }

    #[test]
    fn column_is_per_line_when_only_the_second_line_is_non_ascii() {
        let point = position_of("SELECT 1,\n  '日本', x FROM t", "x");
        assert_eq!(point, Point { row: 1, column: 8 });
    }

    #[test]
    fn point_at_never_panics_on_non_boundary_or_out_of_range_bytes() {
        let sql = "a日b";
        let index = LineIndex::new(sql);
        // Byte 2 is inside the 3-byte "日"; byte 99 is past the end.
        assert_eq!(index.point_at(2), Point { row: 0, column: 1 });
        assert_eq!(index.point_at(99), Point { row: 0, column: 3 });
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
            .find(|n| n.kind() == "ASTPathExpression" && n.text(sql) == Some("col1"))
            .unwrap();
        assert_eq!(col1.byte_range(), 7..11);
        // byte 10 is the first byte of the 3-byte 'あ', so 7..11 cuts it.
        assert_eq!(col1.text("abcdefghijあ"), None);
    }

    #[test]
    fn next_named_sibling_walks_to_the_following_column() {
        // In `select a, b`, the two select columns are siblings under the select
        // list; next_named_sibling from `a`'s column reaches `b`'s.
        let sql = "select a, b from t";
        let ast = parse_sql(sql);
        let a = ast
            .pre_order()
            .into_iter()
            .find(|n| n.kind() == "ASTSelectColumn" && n.text(sql) == Some("a"))
            .unwrap();
        let next = a.next_named_sibling().expect("a has a named sibling b");
        assert_eq!(next.text(sql), Some("b"));
    }

    #[test]
    fn derive_span_ignores_locationless_children() {
        // The span of a rangeless node must cover only its children that carry
        // a real location. Children with the degenerate empty range `0..0` (a
        // subtree ZetaSQL reported no range for) must NOT drag the span to
        // byte 0.
        let mixed = [0..0, 10..14, 0..0, 20..25];
        assert_eq!(derive_span(mixed.iter()), Some(10..25));

        // No child carries a location -> no span can be derived.
        let none = [0..0, 0..0];
        assert_eq!(derive_span(none.iter()), None);

        // No children at all -> no span.
        let empty: [Range<usize>; 0] = [];
        assert_eq!(derive_span(empty.iter()), None);
    }
}

/// Tests for the googlesql/ZetaSQL backend. These exercise
/// [`Ast::from_googlesql`] directly, asserting the arena shape ZetaSQL produces.
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

    /// A fresh googlesql module on the compile-time-selected backend. Building it
    /// links/loads the ZetaSQL parser, so each test pays a small one-time cost;
    /// reuse the returned module across as many parses as the test needs.
    fn module() -> Module {
        crate::build_module().expect("load googlesql module")
    }

    #[test]
    fn from_googlesql_uses_zetasql_kind_names() {
        // The googlesql arena speaks ZetaSQL's vocabulary (AST* class names).
        let sql = "select a from t";
        let ast = Ast::from_googlesql(&mut module(), sql).expect("parse");
        let kinds: Vec<&str> = ast.pre_order().iter().map(NodeRef::kind).collect();
        assert!(kinds.contains(&"ASTSelect"), "kinds were: {kinds:?}");
        assert!(kinds.contains(&"ASTFromClause"), "kinds were: {kinds:?}");
        assert!(kinds.contains(&"ASTIdentifier"), "kinds were: {kinds:?}");
    }

    #[test]
    fn byte_ranges_extract_the_right_source_text() {
        // Every node's byte range must slice back to its own source text, so
        // rules can read text from the source directly.
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
        // synthesizes both.
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
        // ZetaSQL's parse_statement accepts a single statement only; scripts are
        // driven statement-by-statement via parse_statements in the analysis path.
        let err = Ast::from_googlesql(&mut module(), "select 1; select 2;");
        assert!(err.is_err(), "multiple statements must be rejected");
    }
}
