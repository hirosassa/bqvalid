use tree_sitter::{Node, Tree};
use tree_sitter_traversal::{Order, traverse};

use crate::diagnostic::Diagnostic;
use crate::rules::helpers::{find_child_of_kind, has_child_of_kind};

pub fn check(tree: &Tree, sql: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for node in traverse(tree.root_node().walk(), Order::Pre) {
        if matches!(node.kind(), "cte" | "select_subexpression")
            && let Some(diagnostic) = check_unnecessary_order_by_in_scope(&node, sql)
        {
            diagnostics.push(diagnostic);
        }
    }

    diagnostics
}

fn check_unnecessary_order_by_in_scope(scope_node: &Node, _sql: &str) -> Option<Diagnostic> {
    let query_expr = find_query_expr(scope_node)?;

    if !has_child_of_kind(&query_expr, "limit_clause")
        && let Some(order_by_node) = find_child_of_kind(&query_expr, "order_by_clause")
    {
        return Some(new_unnecessary_order_by_warning(&order_by_node));
    }

    None
}

fn find_query_expr<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    node.named_children(&mut node.walk())
        .find(|&child| child.kind() == "query_expr")
}

fn new_unnecessary_order_by_warning(order_by_node: &Node) -> Diagnostic {
    Diagnostic::new(
        order_by_node.start_position().row + 1,
        order_by_node.start_position().column + 1,
        "Unnecessary ORDER BY: This ORDER BY clause has no effect without LIMIT/OFFSET or in aggregate functions".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::helpers::parse_sql;
    use rstest::rstest;
    use std::fs;

    #[rstest]
    #[case("./sql/unnecessary_order_by_in_cte.sql", 1)]
    #[case("./sql/unnecessary_order_by_in_subquery.sql", 1)]
    fn test_unnecessary_order_by_exists(#[case] sql_file: &str, #[case] expected_count: usize) {
        let sql = fs::read_to_string(sql_file).unwrap();
        let tree = parse_sql(&sql);

        let diagnostics = check(&tree, &sql);
        assert_eq!(diagnostics.len(), expected_count);
    }

    #[test]
    fn test_valid_order_by_with_limit() {
        let sql = fs::read_to_string("./sql/valid_order_by_with_limit.sql").unwrap();
        let tree = parse_sql(&sql);

        let diagnostics = check(&tree, &sql);
        assert!(diagnostics.is_empty());
    }
}
