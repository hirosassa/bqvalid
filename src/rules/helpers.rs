use crate::ast::NodeRef;

/// Extract text content from a node, if its byte range maps to valid UTF-8
/// within `sql`.
///
/// Returns `None` when extraction fails. In normal use `sql` is a valid `&str`
/// and the node's byte range points into that same source, so this cannot fail;
/// a `None` therefore signals a corrupted tree or a tree/source mismatch.
pub fn try_get_node_text<'a>(node: &NodeRef<'_>, sql: &'a str) -> Option<&'a str> {
    node.text(sql)
}

/// Extract text content from a node.
///
/// On the (practically impossible) extraction failure this logs a warning and
/// falls back to an empty string, which yields no match downstream (a miss,
/// never a false positive or a crash). The log makes the resulting false
/// negative detectable instead of silently swallowing the error.
pub fn get_node_text<'a>(node: &NodeRef<'_>, sql: &'a str) -> &'a str {
    try_get_node_text(node, sql).unwrap_or_else(|| {
        let (row, col) = one_based_start(node);
        log::warn!(
            "failed to extract text for node (kind: {}) at {row}:{col}; \
             falling back to empty string, which may hide a violation",
            node.kind(),
        );
        ""
    })
}

/// 1-based (row, col) of a node's start position.
///
/// The arena reports positions as 0-based; diagnostics report them as 1-based.
/// This centralizes that `+1` conversion so rules don't each repeat it.
pub fn one_based_start(node: &NodeRef<'_>) -> (usize, usize) {
    let point = node.start_position();
    (point.row.saturating_add(1), point.column.saturating_add(1))
}

/// Find the first named child node with the specified kind
pub fn find_child_of_kind<'a>(node: &NodeRef<'a>, kind: &str) -> Option<NodeRef<'a>> {
    node.named_children()
        .into_iter()
        .find(|child| child.kind() == kind)
}

/// Check if a node has a named child with the specified kind
pub fn has_child_of_kind(node: &NodeRef<'_>, kind: &str) -> bool {
    node.named_children()
        .into_iter()
        .any(|child| child.kind() == kind)
}

/// Find the nearest parent SELECT node (`ASTSelect`).
pub fn find_parent_select<'a>(node: &NodeRef<'a>) -> Option<NodeRef<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "ASTSelect" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Check if a node is a function name (the name part of a function call).
///
/// On googlesql the name identifier is wrapped in an `ASTPathExpression` that is
/// the first child of the `ASTFunctionCall`, so the parent is the path
/// expression rather than the call itself.
pub fn is_function_name(node: &NodeRef<'_>) -> bool {
    let Some(parent) = node.parent() else {
        return false;
    };
    match parent.kind() {
        // Name identifier: `ASTFunctionCall -> ASTPathExpression(name) ->
        // ASTIdentifier` (this node is the identifier).
        "ASTPathExpression" => parent.parent().is_some_and(|grandparent| {
            grandparent.kind() == "ASTFunctionCall"
                && grandparent
                    .child(0)
                    .is_some_and(|first| first.id() == parent.id())
        }),
        // Name path expression itself: it is the first child of the
        // `ASTFunctionCall`. (Rules that navigate column references match the
        // path expression rather than the inner identifier, so this arm skips
        // the function name in that representation.)
        "ASTFunctionCall" => parent.child(0).is_some_and(|first| first.id() == node.id()),
        _ => false,
    }
}

/// Parse `sql` into a neutral [`crate::ast::Ast`] via the googlesql (ZetaSQL)
/// backend (test helper).
///
/// Building a `Module` links the prebuilt ZetaSQL shared library, so keep tests
/// focused rather than parsing the same SQL many times.
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]
pub fn parse_sql(sql: &str) -> crate::ast::Ast {
    use googlesql::Module;

    let mut module = Module::new_native_ffi().expect("googlesql module builds");
    crate::ast::Ast::from_googlesql(&mut module, sql).expect("googlesql parses the sql")
}

/// Parse `sql` and run a single rule over it, returning its diagnostics.
///
/// Collapses the repeated `parse_sql(...)` + `rule.check(&ast, sql)` boilerplate
/// in rule tests so each case can pass an inline SQL literal and assert on the
/// result directly.
#[cfg(test)]
pub fn run_rule<R: crate::rules::rule::Rule>(
    rule: &R,
    sql: &str,
) -> Vec<crate::diagnostic::Diagnostic> {
    let ast = parse_sql(sql);
    rule.check(&ast, sql)
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
    use crate::ast::NodeRef;

    /// The `col1` column-reference node in `sql`, for tests that need a concrete
    /// node with a known byte range. On googlesql a column reference is an
    /// `ASTPathExpression` whose text is the (possibly qualified) name.
    fn col1_ref<'a>(ast: &'a crate::ast::Ast, sql: &str) -> Option<NodeRef<'a>> {
        ast.pre_order()
            .into_iter()
            .find(|node| node.kind() == "ASTPathExpression" && get_node_text(node, sql) == "col1")
    }

    #[test]
    fn test_get_node_text() {
        let sql = "SELECT col1 FROM table1";
        let ast = parse_sql(sql);
        let node = col1_ref(&ast, sql).expect("Should find the col1 reference");
        assert_eq!(get_node_text(&node, sql), "col1");
    }

    #[test]
    fn try_get_node_text_returns_some_on_valid_source() {
        let sql = "SELECT col1 FROM t";
        let ast = parse_sql(sql);
        let node = col1_ref(&ast, sql).expect("expected to find the col1 reference");
        assert_eq!(try_get_node_text(&node, sql), Some("col1"));
    }

    #[test]
    fn try_get_node_text_returns_none_when_range_splits_a_multibyte_char() {
        // Simulate a corrupted tree / source mismatch: extract a node's byte
        // range from a *different* source in which that range slices through a
        // multibyte character, yielding invalid UTF-8. `text` then returns None
        // instead of silently producing an empty string (a hidden false negative).
        let sql = "SELECT col1 FROM t";
        let ast = parse_sql(sql);
        let node = col1_ref(&ast, sql).expect("expected to find the col1 reference");
        // "col1" occupies bytes 7..11 in `sql`. Build a same-or-longer valid
        // string in which byte 10 is the first byte of a 3-byte character, so
        // that slicing 7..11 cuts it and produces invalid UTF-8.
        let mismatched = "abcdefghijあ";
        assert_eq!(node.start_byte(), 7);
        assert_eq!(node.end_byte(), 11);
        assert_eq!(try_get_node_text(&node, mismatched), None);
    }

    #[test]
    fn get_node_text_falls_back_to_empty_string_on_failure() {
        // The public helper keeps its `&str` signature and returns "" on the
        // (logged) failure path, so downstream comparisons simply miss.
        let sql = "SELECT col1 FROM t";
        let ast = parse_sql(sql);
        let node = col1_ref(&ast, sql).expect("expected to find the col1 reference");
        let mismatched = "abcdefghijあ";
        assert_eq!(get_node_text(&node, mismatched), "");
    }

    #[test]
    fn one_based_start_converts_zero_based_position() {
        // The single column reference starts at row 0, col 7 (0-based);
        // one_based_start must report it as (1, 8).
        let sql = "SELECT col1 FROM t";
        let ast = parse_sql(sql);
        let node = col1_ref(&ast, sql).expect("expected to find the col1 reference");
        assert_eq!(one_based_start(&node), (1, 8));
    }

    #[test]
    fn test_find_child_of_kind() {
        let sql = "SELECT col1 FROM table1 GROUP BY col1";
        let ast = parse_sql(sql);
        let select = ast
            .pre_order()
            .into_iter()
            .find(|node| node.kind() == "ASTSelect")
            .expect("Should find select node");

        // The select node should have an ASTGroupBy child
        let group_by = find_child_of_kind(&select, "ASTGroupBy");
        assert!(group_by.is_some(), "Should find ASTGroupBy node");

        // Should return None for non-existent kind
        let non_existent = find_child_of_kind(&select, "non_existent_kind");
        assert!(non_existent.is_none(), "Should not find non-existent kind");
    }

    #[test]
    fn test_has_child_of_kind() {
        let sql = "SELECT col1 FROM table1 GROUP BY col1";
        let ast = parse_sql(sql);
        let select = ast
            .pre_order()
            .into_iter()
            .find(|node| node.kind() == "ASTSelect")
            .expect("Should find select node");

        // The select node should have an ASTGroupBy child
        assert!(
            has_child_of_kind(&select, "ASTGroupBy"),
            "Should have ASTGroupBy child"
        );

        // Should return false for non-existent kind
        assert!(
            !has_child_of_kind(&select, "non_existent_kind"),
            "Should not have non-existent kind"
        );
    }
}
