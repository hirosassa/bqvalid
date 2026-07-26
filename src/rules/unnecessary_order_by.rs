use tree_sitter::Node;

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::helpers::{find_child_of_kind, has_child_of_kind, one_based_start};
use crate::rules::rule::Rule;

const RULE_ID: &str = "unnecessary_order_by";

/// Flags `ORDER BY` in a CTE or subquery without `LIMIT`, where the sort has no
/// effect and only wastes work.
pub struct UnnecessaryOrderBy;

impl Rule for UnnecessaryOrderBy {
    fn id(&self) -> &'static str {
        RULE_ID
    }

    fn check_node(&self, node: Node<'_>, sql: &str, diagnostics: &mut Vec<Diagnostic>) {
        if matches!(node.kind(), "cte" | "select_subexpression")
            && let Some(diagnostic) = check_unnecessary_order_by_in_scope(&node, sql)
        {
            diagnostics.push(diagnostic);
        }
    }
}

fn check_unnecessary_order_by_in_scope(scope_node: &Node, _sql: &str) -> Option<Diagnostic> {
    let query_expr = find_query_expr(scope_node)?;

    if !has_child_of_kind(&query_expr, "limit_clause")
        && let Some(order_by_node) = find_child_of_kind(&query_expr, "order_by_clause")
    {
        let (row, col) = one_based_start(&order_by_node);
        return Some(Diagnostic::new(
            RULE_ID,
            Severity::Warning,
            row,
            col,
            "Unnecessary ORDER BY: This ORDER BY clause has no effect without LIMIT/OFFSET or in aggregate functions".to_string(),
        ));
    }

    None
}

fn find_query_expr<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    node.named_children(&mut node.walk())
        .find(|&child| child.kind() == "query_expr")
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
    use rstest::rstest;
    use std::fs;

    #[rstest]
    #[case("./sql/unnecessary_order_by_in_cte.sql", 1)]
    #[case("./sql/unnecessary_order_by_in_subquery.sql", 1)]
    fn test_unnecessary_order_by_exists(#[case] sql_file: &str, #[case] expected_count: usize) {
        let sql = fs::read_to_string(sql_file).unwrap();
        let tree = parse_sql(&sql);

        let diagnostics = UnnecessaryOrderBy.check(&tree, &sql);
        assert_eq!(diagnostics.len(), expected_count);
    }

    #[test]
    fn test_valid_order_by_with_limit() {
        let sql = fs::read_to_string("./sql/valid_order_by_with_limit.sql").unwrap();
        let tree = parse_sql(&sql);

        let diagnostics = UnnecessaryOrderBy.check(&tree, &sql);
        assert!(diagnostics.is_empty());
    }
}
