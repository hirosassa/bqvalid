use tree_sitter::Node;

/// Extract text content from a tree-sitter node
pub fn get_node_text<'a>(node: &Node, sql: &'a str) -> &'a str {
    node.utf8_text(sql.as_bytes()).unwrap()
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
pub fn parse_sql(sql: &str) -> tree_sitter::Tree {
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;

    let mut parser = TsParser::new();
    parser.set_language(&language()).unwrap();
    parser.parse(sql, None).unwrap()
}

#[cfg(test)]
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
