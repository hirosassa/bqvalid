use tree_sitter::Node;
use tree_sitter_traversal::{Order, traverse};

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::helpers::{get_node_text, one_based_start};
use crate::rules::rule::Rule;

const RULE_ID: &str = "compare_table_suffix_with_subquery";

/// Flags `_TABLE_SUFFIX` compared against a subquery, which forces a full scan.
pub struct CompareTableSuffixWithSubquery;

impl Rule for CompareTableSuffixWithSubquery {
    fn id(&self) -> &'static str {
        RULE_ID
    }

    fn check_node(&self, node: Node<'_>, sql: &str, diagnostics: &mut Vec<Diagnostic>) {
        if node.kind() == "where_clause" {
            if let Some(diagnostic) = compared_with_subquery_in_binary_expression(node, sql) {
                diagnostics.push(diagnostic);
            }
            if let Some(diagnostic) = compared_with_subquery_in_between_expression(node, sql) {
                diagnostics.push(diagnostic);
            }
        }
    }
}

fn compared_with_subquery_in_binary_expression(n: Node, src: &str) -> Option<Diagnostic> {
    for node in traverse(n.walk(), Order::Pre) {
        let text = get_node_text(&node, src);

        if node.kind() == "identifier" && text.eq_ignore_ascii_case("_table_suffix") {
            let Some(parent) = node.parent() else {
                continue;
            };
            let mut tc = parent.walk();
            let Some(right_operand) = parent.children(&mut tc).last() else {
                continue;
            };
            if parent.kind() == "binary_expression"
                && right_operand.kind() == "select_subexpression"
            {
                return Some(new_full_scan_warning(&right_operand));
            }
        }
    }
    None
}

fn compared_with_subquery_in_between_expression(n: Node, src: &str) -> Option<Diagnostic> {
    for node in traverse(n.walk(), Order::Pre) {
        let text = get_node_text(&node, src);

        if node.kind() == "identifier" && text.eq_ignore_ascii_case("_table_suffix") {
            let Some(parent) = node.parent() else {
                continue;
            };
            if parent.kind() == "between_operator" {
                let mut tc = parent.walk();
                for c in parent.children(&mut tc) {
                    let Some(first_child) = c.child(0) else {
                        continue;
                    };
                    if (c.kind() == "between_from" || c.kind() == "between_to")
                        && first_child.kind() == "select_subexpression"
                    {
                        return Some(new_full_scan_warning(&first_child));
                    }
                }
            }
        }
    }
    None
}

/// Build the full-scan diagnostic pointing at `subquery_node`. Both the binary
/// and BETWEEN paths report the same problem, so construction lives here.
fn new_full_scan_warning(subquery_node: &Node) -> Diagnostic {
    let (row, col) = one_based_start(subquery_node);
    Diagnostic::new(
        RULE_ID,
        Severity::Warning,
        row,
        col,
        "Full scan will cause! Should not compare _TABLE_SUFFIX with subquery".to_string(),
    )
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
    use std::fs;

    #[test]
    fn valid() {
        let sql = fs::read_to_string("./sql/valid.sql").unwrap();
        let tree = parse_sql(&sql);

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_binary_expression(node, &sql).is_none());
                assert!(compared_with_subquery_in_between_expression(node, &sql).is_none());
            }
        }
    }

    #[test]
    fn binary_op() {
        let sql = fs::read_to_string("./sql/subquery_with_binary_op.sql").unwrap();
        let tree = parse_sql(&sql);

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_binary_expression(node, &sql).is_some());
            }
        }
    }

    #[test]
    fn between_from() {
        let sql = fs::read_to_string("./sql/subquery_with_between_from.sql").unwrap();
        let tree = parse_sql(&sql);

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_between_expression(node, &sql).is_some());
            }
        }
    }

    #[test]
    fn between_to() {
        let sql = fs::read_to_string("./sql/subquery_with_between_to.sql").unwrap();
        let tree = parse_sql(&sql);

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_between_expression(node, &sql).is_some());
            }
        }
    }

    #[test]
    fn binary_op_points_at_the_subquery_position() {
        // The diagnostic must point at the offending subquery, not the WHERE
        // clause. On this single-line query the subquery starts at the '('.
        let sql = "SELECT x FROM t WHERE _TABLE_SUFFIX = (SELECT MAX(s) FROM u)";
        let paren_col = sql.find('(').expect("query contains a subquery paren");
        let tree = parse_sql(sql);

        let mut checked = false;
        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                let diagnostic = compared_with_subquery_in_binary_expression(node, sql)
                    .expect("subquery comparison should be flagged");
                // 1-based, and row 1 because the query is on a single line.
                assert_eq!(diagnostic.row(), 1);
                assert_eq!(diagnostic.col(), paren_col + 1);
                checked = true;
            }
        }
        assert!(checked, "a where_clause node should exist");
    }

    #[test]
    fn does_not_panic_on_malformed_where_clause() {
        // Truncated / malformed input must not panic the node-navigation logic.
        for sql in [
            "SELECT x FROM t WHERE _TABLE_SUFFIX",
            "WHERE _TABLE_SUFFIX = (",
        ] {
            let tree = parse_sql(sql);
            for node in traverse(tree.walk(), Order::Pre) {
                if node.kind() == "where_clause" {
                    let _ = compared_with_subquery_in_binary_expression(node, sql);
                    let _ = compared_with_subquery_in_between_expression(node, sql);
                }
            }
        }
    }
}
