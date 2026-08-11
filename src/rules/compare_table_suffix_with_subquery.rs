use crate::ast::NodeRef;

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

    fn check_node(&self, node: NodeRef<'_>, sql: &str, diagnostics: &mut Vec<Diagnostic>) {
        if node.kind() == "ASTWhereClause" {
            if let Some(diagnostic) = compared_with_subquery_in_binary_expression(node, sql) {
                diagnostics.push(diagnostic);
            }
            if let Some(diagnostic) = compared_with_subquery_in_between_expression(node, sql) {
                diagnostics.push(diagnostic);
            }
        }
    }
}

/// True when `node` is a `_TABLE_SUFFIX` reference: the `ASTPathExpression`
/// wrapping the identifier on googlesql (whose text is the single name).
fn is_table_suffix(node: &NodeRef<'_>, src: &str) -> bool {
    node.kind() == "ASTPathExpression"
        && get_node_text(node, src).eq_ignore_ascii_case("_table_suffix")
}

/// True when `node` is a scalar subquery (`ASTExpressionSubquery` on googlesql).
fn is_subquery(node: &NodeRef<'_>) -> bool {
    node.kind() == "ASTExpressionSubquery"
}

fn compared_with_subquery_in_binary_expression(n: NodeRef<'_>, src: &str) -> Option<Diagnostic> {
    for node in n.pre_order() {
        if node.kind() != "ASTBinaryExpression" {
            continue;
        }
        // `_TABLE_SUFFIX <op> (subquery)`: the left operand is the first child and
        // the compared-against subquery is the last.
        let Some(left) = node.child(0) else {
            continue;
        };
        let Some(right) = node.children().into_iter().last() else {
            continue;
        };
        if is_table_suffix(&left, src) && is_subquery(&right) {
            return Some(new_full_scan_warning(&right));
        }
    }
    None
}

fn compared_with_subquery_in_between_expression(n: NodeRef<'_>, src: &str) -> Option<Diagnostic> {
    for node in n.pre_order() {
        if node.kind() != "ASTBetweenExpression" {
            continue;
        }
        let Some(operand) = node.child(0) else {
            continue;
        };
        if !is_table_suffix(&operand, src) {
            continue;
        }
        // googlesql lists the bounds directly under ASTBetweenExpression.
        for c in node.children() {
            if is_subquery(&c) {
                return Some(new_full_scan_warning(&c));
            }
        }
    }
    None
}

/// Build the full-scan diagnostic pointing at `subquery_node`. Both the binary
/// and BETWEEN paths report the same problem, so construction lives here.
fn new_full_scan_warning(subquery_node: &NodeRef<'_>) -> Diagnostic {
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

    #[test]
    fn valid() {
        let sql = "\
select
  *
from
  dataset.table
";
        let ast = parse_sql(sql);

        for node in ast.pre_order() {
            if node.kind() == "ASTWhereClause" {
                assert!(compared_with_subquery_in_binary_expression(node, sql).is_none());
                assert!(compared_with_subquery_in_between_expression(node, sql).is_none());
            }
        }
    }

    #[test]
    fn binary_op() {
        let sql = "\
select
  *
from
  dataset.table
where
  _table_suffix  = (
    select dt from dates
  )
";
        let ast = parse_sql(sql);

        for node in ast.pre_order() {
            if node.kind() == "ASTWhereClause" {
                assert!(compared_with_subquery_in_binary_expression(node, sql).is_some());
            }
        }
    }

    #[test]
    fn between_from() {
        let sql = "\
select
  *
from
  dataset.table
where
  _table_suffix between (
    select dt from dates
  )
  and '2022-06-01'
";
        let ast = parse_sql(sql);

        for node in ast.pre_order() {
            if node.kind() == "ASTWhereClause" {
                assert!(compared_with_subquery_in_between_expression(node, sql).is_some());
            }
        }
    }

    #[test]
    fn between_to() {
        let sql = "\
select
  *
from
  dataset.table
where
  _table_suffix between '2022-06-01'
  and (
    select dt from dates
  )
";
        let ast = parse_sql(sql);

        for node in ast.pre_order() {
            if node.kind() == "ASTWhereClause" {
                assert!(compared_with_subquery_in_between_expression(node, sql).is_some());
            }
        }
    }

    #[test]
    fn binary_op_points_at_the_subquery_position() {
        // The diagnostic must point at the offending subquery, not the WHERE
        // clause. On this single-line query the subquery starts at the '('.
        let sql = "SELECT x FROM t WHERE _TABLE_SUFFIX = (SELECT MAX(s) FROM u)";
        let paren_col = sql.find('(').expect("query contains a subquery paren");
        let ast = parse_sql(sql);

        let mut checked = false;
        for node in ast.pre_order() {
            if node.kind() == "ASTWhereClause" {
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
    fn comparing_against_a_literal_is_not_flagged() {
        let sql = "SELECT x FROM t WHERE _TABLE_SUFFIX = '2022-06-01'";
        let ast = parse_sql(sql);
        assert!(
            CompareTableSuffixWithSubquery.check(&ast, sql).is_empty(),
            "comparing against a literal must not be flagged"
        );
    }

    #[test]
    fn does_not_panic_on_non_subquery_comparisons() {
        // WHERE shapes that do not compare _TABLE_SUFFIX against a subquery must
        // not panic the node navigation (and must not be flagged).
        for sql in [
            "SELECT x FROM t WHERE _TABLE_SUFFIX = '2022-06-01'",
            "SELECT x FROM t WHERE _TABLE_SUFFIX BETWEEN '2022-06-01' AND '2022-06-02'",
        ] {
            let ast = parse_sql(sql);
            for node in ast.pre_order() {
                if node.kind() == "ASTWhereClause" {
                    let _ = compared_with_subquery_in_binary_expression(node, sql);
                    let _ = compared_with_subquery_in_between_expression(node, sql);
                }
            }
        }
    }
}
