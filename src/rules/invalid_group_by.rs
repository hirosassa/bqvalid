use std::collections::HashSet;
use tree_sitter::{Node, Tree};
use tree_sitter_traversal::{Order, traverse};

use crate::diagnostic::Diagnostic;

pub fn check(tree: &Tree, sql: &str) -> Option<Vec<Diagnostic>> {
    let diagnostics = find_invalid_group_by(tree, sql);

    if diagnostics.is_empty() {
        None
    } else {
        Some(diagnostics)
    }
}

fn find_invalid_group_by(tree: &Tree, sql: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for node in traverse(tree.walk(), Order::Pre) {
        if node.kind() == "select"
            && let Some(diags) = check_select(&node, sql)
        {
            diagnostics.extend(diags);
        }
    }

    diagnostics
}

fn check_select(node: &Node, sql: &str) -> Option<Vec<Diagnostic>> {
    let group_by_columns = extract_group_by_columns(node, sql)?;

    let select_list = find_child_of_kind(node, "select_list")?;

    let mut diagnostics = Vec::new();
    for child in select_list.named_children(&mut select_list.walk()) {
        if child.kind() == "select_expression"
            && let Some(diag) = check_select_expression(&child, sql, &group_by_columns)
        {
            diagnostics.push(diag);
        }
    }

    if diagnostics.is_empty() {
        None
    } else {
        Some(diagnostics)
    }
}

fn find_child_of_kind<'a>(node: &'a Node<'a>, kind: &str) -> Option<Node<'a>> {
    node.named_children(&mut node.walk())
        .find(|child| child.kind() == kind)
}

fn extract_group_by_columns(select_node: &Node, sql: &str) -> Option<HashSet<String>> {
    let group_by_node = find_child_of_kind(select_node, "group_by_clause")?;
    let mut columns = HashSet::new();

    for node in traverse(group_by_node.walk(), Order::Pre) {
        if node.kind() == "identifier" {
            let text = get_node_text(&node, sql);
            columns.insert(text);
        }
    }

    Some(columns)
}

fn check_select_expression(
    expr_node: &Node,
    sql: &str,
    group_by_columns: &HashSet<String>,
) -> Option<Diagnostic> {
    // Check if this expression contains an identifier that's not in an aggregate function
    for node in traverse(expr_node.walk(), Order::Pre) {
        if node.kind() == "identifier" && !is_alias(&node) && !is_in_aggregate_function(&node, sql)
        {
            let field_text = get_node_text(&node, sql);

            // Check if the identifier is in GROUP BY
            if !group_by_columns.contains(&field_text) {
                return Some(Diagnostic::new(
                    node.start_position().row + 1,
                    node.start_position().column + 1,
                    format!(
                        "Column '{}' must appear in the GROUP BY clause or be used in an aggregate function",
                        field_text
                    ),
                ));
            }
        }
    }

    None
}

fn is_alias(node: &Node) -> bool {
    // Check if this identifier is part of an as_alias
    if let Some(parent) = node.parent()
        && parent.kind() == "as_alias"
    {
        return true;
    }
    false
}

fn is_in_aggregate_function(node: &Node, sql: &str) -> bool {
    let mut current = node.parent();

    while let Some(parent) = current {
        if parent.kind() == "function_call" {
            // Get the function name from the 'function' field
            if let Some(func_node) = parent.child_by_field_name("function") {
                let func_name = get_node_text(&func_node, sql);

                // Common aggregate functions in BigQuery (case-insensitive)
                let func_name_upper = func_name.to_uppercase();
                if matches!(
                    func_name_upper.as_str(),
                    "COUNT"
                        | "SUM"
                        | "AVG"
                        | "MAX"
                        | "MIN"
                        | "ANY_VALUE"
                        | "ARRAY_AGG"
                        | "STRING_AGG"
                        | "COUNTIF"
                ) {
                    return true;
                }
            }
        }
        current = parent.parent();
    }

    false
}

fn get_node_text(node: &Node, sql: &str) -> String {
    let range = node.range();
    sql[range.start_byte..range.end_byte].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;

    fn parse_sql(sql: &str) -> tree_sitter::Tree {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();
        parser.parse(sql, None).unwrap()
    }

    #[rstest]
    #[case("invalid_group_by_column_not_in_clause.sql", 1)]
    #[case("invalid_group_by_multiple_violations.sql", 2)]
    #[case("invalid_group_by_in_subquery.sql", 1)]
    fn test_invalid_group_by(#[case] filename: &str, #[case] expected_count: usize) {
        let sql = fs::read_to_string(format!("./sql/{}", filename)).unwrap();
        let tree = parse_sql(&sql);

        let result = check(&tree, &sql);
        assert!(
            result.is_some(),
            "Expected to detect invalid GROUP BY in {}",
            filename
        );
        let diagnostics = result.unwrap();
        assert_eq!(
            diagnostics.len(),
            expected_count,
            "Expected {} diagnostic(s) in {}, got {}",
            expected_count,
            filename,
            diagnostics.len()
        );
    }

    #[rstest]
    #[case("valid_group_by_with_aggregates.sql")]
    #[case("valid_group_by_mixed_case_aggregates.sql")]
    fn test_valid_group_by(#[case] filename: &str) {
        let sql = fs::read_to_string(format!("./sql/{}", filename)).unwrap();
        let tree = parse_sql(&sql);

        let result = check(&tree, &sql);
        assert!(
            result.is_none(),
            "Expected no diagnostics for valid GROUP BY in {}",
            filename
        );
    }

    #[test]
    fn test_is_alias() {
        let sql = "SELECT col1 as alias1 FROM table1";
        let tree = parse_sql(sql);

        // Find the identifier "alias1"
        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "identifier" {
                let text = get_node_text(&node, sql);
                if text == "alias1" {
                    assert!(is_alias(&node), "alias1 should be recognized as an alias");
                } else if text == "col1" {
                    assert!(
                        !is_alias(&node),
                        "col1 should not be recognized as an alias"
                    );
                }
            }
        }
    }

    #[test]
    fn test_is_in_aggregate_function() {
        let sql = "SELECT COUNT(col1), col2 FROM table1 GROUP BY col2";
        let tree = parse_sql(sql);

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "identifier" {
                let text = get_node_text(&node, sql);
                if text == "col1" {
                    assert!(
                        is_in_aggregate_function(&node, sql),
                        "col1 should be recognized as inside aggregate function"
                    );
                }
            }
        }
    }
}
