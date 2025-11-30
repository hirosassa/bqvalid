use tree_sitter::{Node, Tree};
use tree_sitter_traversal::{Order, traverse};

use crate::diagnostic::Diagnostic;

pub fn check(tree: &Tree, sql: &str) -> Option<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    let root = tree.root_node();

    for cte_node in find_ctes(&root) {
        if let Some(diagnostic) = check_unnecessary_order_by_in_scope(&cte_node, sql) {
            diagnostics.push(diagnostic);
        }
    }

    for subquery_node in find_subqueries(&root) {
        if let Some(diagnostic) = check_unnecessary_order_by_in_scope(&subquery_node, sql) {
            diagnostics.push(diagnostic);
        }
    }

    if diagnostics.is_empty() {
        None
    } else {
        Some(diagnostics)
    }
}

fn find_ctes<'a>(node: &'a Node<'a>) -> Vec<Node<'a>> {
    let mut cte_nodes = Vec::new();
    for n in traverse(node.walk(), Order::Pre) {
        if n.kind() == "cte" {
            cte_nodes.push(n);
        }
    }
    cte_nodes
}

fn find_subqueries<'a>(node: &'a Node<'a>) -> Vec<Node<'a>> {
    let mut subquery_nodes = Vec::new();
    for n in traverse(node.walk(), Order::Pre) {
        if n.kind() == "select_subexpression" {
            subquery_nodes.push(n);
        }
    }
    subquery_nodes
}

fn check_unnecessary_order_by_in_scope(scope_node: &Node, _sql: &str) -> Option<Diagnostic> {
    let query_expr = find_query_expr(scope_node)?;

    let has_order_by = has_node_of_kind(&query_expr, "order_by_clause");
    let has_limit = has_node_of_kind(&query_expr, "limit_clause");

    if has_order_by
        && !has_limit
        && let Some(order_by_node) = find_node_of_kind(&query_expr, "order_by_clause")
    {
        return Some(new_unnecessary_order_by_warning(&order_by_node));
    }

    None
}

fn find_query_expr<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    node.named_children(&mut node.walk())
        .find(|&child| child.kind() == "query_expr")
}

fn has_node_of_kind(node: &Node, kind: &str) -> bool {
    for child in node.named_children(&mut node.walk()) {
        if child.kind() == kind {
            return true;
        }
    }
    false
}

fn find_node_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    node.named_children(&mut node.walk())
        .find(|&child| child.kind() == kind)
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
    use rstest::rstest;
    use std::fs;
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;

    #[rstest]
    #[case("./sql/unnecessary_order_by_in_cte.sql", 1)]
    #[case("./sql/unnecessary_order_by_in_subquery.sql", 1)]
    fn test_unnecessary_order_by_exists(#[case] sql_file: &str, #[case] expected_count: usize) {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string(sql_file).unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        let diagnostics = check(&tree, &sql);
        assert!(diagnostics.is_some());
        assert_eq!(diagnostics.unwrap().len(), expected_count);
    }

    #[test]
    fn test_valid_order_by_with_limit() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/valid_order_by_with_limit.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        let diagnostics = check(&tree, &sql);
        assert!(diagnostics.is_none());
    }
}
