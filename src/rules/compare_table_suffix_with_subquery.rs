use tree_sitter::{Node, Tree};
use tree_sitter_traversal::{traverse, Order};

use crate::diagnostic::Diagnostic;

pub fn check(tree: &Tree, sql: &str) -> Option<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    for node in traverse(tree.walk(), Order::Pre) {
        if node.kind() == "where_clause" {
            if let Some(diagnostic) = compared_with_subquery_in_binary_expression(node, sql) {
                diagnostics.push(diagnostic);
            }
            if let Some(diagnostic) = compared_with_subquery_in_between_expression(node, sql) {
                diagnostics.push(diagnostic);
            }
        }
    }

    if diagnostics.is_empty() {
        None
    } else {
        Some(diagnostics)
    }
}

fn compared_with_subquery_in_binary_expression(n: Node, src: &str) -> Option<Diagnostic> {
    for node in traverse(n.walk(), Order::Pre) {
        let range = node.range();
        let text = &src[range.start_byte..range.end_byte];

        if node.kind() == "identifier" && text.to_ascii_lowercase() == "_table_suffix" {
            let parent = node.parent().unwrap();
            let mut tc = parent.walk();
            let right_operand = parent.children(&mut tc).last().unwrap();
            if parent.kind() == "binary_expression"
                && right_operand.kind() == "select_subexpression"
            {
                let rg = right_operand.range();
                return Some(new_full_scan_warning(
                    rg.start_point.row,
                    rg.start_point.column,
                ));
            }
        }
    }
    None
}

fn compared_with_subquery_in_between_expression(n: Node, src: &str) -> Option<Diagnostic> {
    for node in traverse(n.walk(), Order::Pre) {
        let range = node.range();
        let text = &src[range.start_byte..range.end_byte];

        if node.kind() == "identifier" && text.to_ascii_lowercase() == "_table_suffix" {
            let parent = node.parent().unwrap();
            if parent.kind() == "between_operator" {
                let mut tc = parent.walk();
                for c in parent.children(&mut tc) {
                    if (c.kind() == "between_from" || c.kind() == "between_to")
                        && c.child(0).unwrap().kind() == "select_subexpression"
                    {
                        let rg = c.child(0).unwrap().range();
                        return Some(new_full_scan_warning(
                            rg.start_point.row,
                            rg.start_point.column,
                        ));
                    }
                }
            }
        }
    }
    None
}

fn new_full_scan_warning(row: usize, col: usize) -> Diagnostic {
    Diagnostic::new(
        row + 1,
        col + 1,
        "Full scan will cause! Should not compare _TABLE_SUFFIX with subquery".to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;

    #[test]
    fn valid() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/valid.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert_eq!(
                    compared_with_subquery_in_binary_expression(node, &sql).is_some(),
                    false
                );
                assert_eq!(
                    compared_with_subquery_in_between_expression(node, &sql).is_some(),
                    false
                );
            }
        }
    }

    #[test]
    fn binary_op() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/subquery_with_binary_op.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_binary_expression(node, &sql).is_some());
            }
        }
    }

    #[test]
    fn between_from() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/subquery_with_between_from.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_between_expression(node, &sql).is_some());
            }
        }
    }

    #[test]
    fn between_to() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/subquery_with_between_to.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_between_expression(node, &sql).is_some());
            }
        }
    }
}
