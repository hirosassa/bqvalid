use tree_sitter::Node;

/// Extract text content from a tree-sitter node.
///
/// `sql` is a valid `&str`, and the node's byte range points into that same
/// source, so `utf8_text` cannot actually fail here. We still avoid panicking:
/// on the impossible error we fall back to an empty string, which yields no
/// match downstream (a miss, never a false positive or a crash).
pub fn get_node_text<'a>(node: &Node, sql: &'a str) -> &'a str {
    node.utf8_text(sql.as_bytes()).unwrap_or_default()
}

/// 1-based (row, col) of a node's start position.
///
/// tree-sitter reports positions as 0-based; diagnostics report them as 1-based.
/// This centralizes that `+1` conversion so rules don't each repeat it.
pub fn one_based_start(node: &Node) -> (usize, usize) {
    let point = node.start_position();
    (point.row.saturating_add(1), point.column.saturating_add(1))
}

/// Find the first child node with the specified kind
pub fn find_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    node.named_children(&mut node.walk())
        .find(|child| child.kind() == kind)
}

/// Check if a node has a child with the specified kind
pub fn has_child_of_kind(node: &Node, kind: &str) -> bool {
    node.named_children(&mut node.walk())
        .any(|child| child.kind() == kind)
}

/// Find the nearest parent node with kind "select"
pub fn find_parent_select<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "select" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Check if a node is a function name (the name part of a function_call)
pub fn is_function_name(node: &Node) -> bool {
    if let Some(parent) = node.parent()
        && parent.kind() == "function_call"
    {
        if let Some(func_node) = parent.child_by_field_name("function") {
            return func_node.id() == node.id();
        }
        if let Some(first_child) = parent.child(0) {
            return first_child.id() == node.id();
        }
    }
    false
}

/// Parse SQL string into a tree-sitter tree (test helper)
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test code"
)]
pub fn parse_sql(sql: &str) -> tree_sitter::Tree {
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;

    let mut parser = TsParser::new();
    parser.set_language(&language()).unwrap();
    parser.parse(sql, None).unwrap()
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

    #[test]
    fn test_get_node_text() {
        let sql = "SELECT col1 FROM table1";
        let tree = parse_sql(sql);

        // Find the first identifier node using traverse
        use tree_sitter_traversal::{Order, traverse};
        let mut found = false;
        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "identifier" {
                let text = get_node_text(&node, sql);
                assert!(text == "col1" || text == "table1" || text == "SELECT");
                found = true;
                break;
            }
        }
        assert!(found, "Should find at least one identifier");
    }

    #[test]
    fn one_based_start_converts_zero_based_position() {
        // The single identifier starts at row 0, col 7 (0-based) in tree-sitter;
        // one_based_start must report it as (1, 8).
        let sql = "SELECT col1 FROM t";
        let tree = parse_sql(sql);

        use tree_sitter_traversal::{Order, traverse};
        let mut checked = false;
        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "identifier" && get_node_text(&node, sql) == "col1" {
                assert_eq!(one_based_start(&node), (1, 8));
                checked = true;
                break;
            }
        }
        assert!(checked, "expected to find the col1 identifier");
    }

    #[test]
    fn test_find_child_of_kind() {
        let sql = "SELECT col1 FROM table1 GROUP BY col1";
        let tree = parse_sql(sql);

        // Find a select node first, then look for its children
        use tree_sitter_traversal::{Order, traverse};
        let mut select_node = None;
        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "select" {
                select_node = Some(node);
                break;
            }
        }

        let select = select_node.unwrap();
        // The select node should have a "group_by_clause" child
        let group_by = find_child_of_kind(&select, "group_by_clause");
        assert!(group_by.is_some(), "Should find group_by_clause node");

        // Should return None for non-existent kind
        let non_existent = find_child_of_kind(&select, "non_existent_kind");
        assert!(non_existent.is_none(), "Should not find non-existent kind");
    }

    #[test]
    fn test_has_child_of_kind() {
        let sql = "SELECT col1 FROM table1 GROUP BY col1";
        let tree = parse_sql(sql);

        // Find a select node first
        use tree_sitter_traversal::{Order, traverse};
        let mut select_node = None;
        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "select" {
                select_node = Some(node);
                break;
            }
        }

        let select = select_node.unwrap();
        // The select node should have a "group_by_clause" child
        assert!(
            has_child_of_kind(&select, "group_by_clause"),
            "Should have group_by_clause child"
        );

        // Should return false for non-existent kind
        assert!(
            !has_child_of_kind(&select, "non_existent_kind"),
            "Should not have non-existent kind"
        );
    }
}
