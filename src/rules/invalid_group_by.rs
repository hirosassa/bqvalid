use std::collections::HashSet;
use tree_sitter::{Node, Tree};
use tree_sitter_traversal::{Order, traverse};

use crate::diagnostic::Diagnostic;
use crate::rules::helpers::{find_child_of_kind, get_node_text};

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
        if node.kind() == "identifier"
            && !is_alias(&node)
            && !is_function_name(&node)
            && !is_in_aggregate_function(&node, sql)
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

fn is_function_name(node: &Node) -> bool {
    // Check if this identifier is a function name
    // Function names are direct children of function_call nodes with field name "function"
    if let Some(parent) = node.parent()
        && parent.kind() == "function_call"
        && let Some(func_node) = parent.child_by_field_name("function")
    {
        return node.id() == func_node.id();
    }
    false
}

fn is_in_aggregate_function(node: &Node, sql: &str) -> bool {
    let mut current = node.parent();

    while let Some(parent) = current {
        if parent.kind() == "function_call"
            && let Some(func_node) = parent.child_by_field_name("function")
        {
            let func_name = get_node_text(&func_node, sql);

            // BigQuery aggregate functions (case-insensitive)
            // Reference: https://cloud.google.com/bigquery/docs/reference/standard-sql/aggregate_functions
            let func_name_upper = func_name.to_uppercase();
            if matches!(
                func_name_upper.as_str(),
                // Standard aggregate functions
                "ANY_VALUE"
                    | "ARRAY_AGG"
                    | "ARRAY_CONCAT_AGG"
                    | "AVG"
                    | "BIT_AND"
                    | "BIT_OR"
                    | "BIT_XOR"
                    | "COUNT"
                    | "COUNTIF"
                    | "GROUPING"
                    | "LOGICAL_AND"
                    | "LOGICAL_OR"
                    | "MAX"
                    | "MAX_BY"
                    | "MIN"
                    | "MIN_BY"
                    | "STRING_AGG"
                    | "SUM"
                    // Approximate aggregate functions
                    | "APPROX_COUNT_DISTINCT"
                    | "APPROX_QUANTILES"
                    | "APPROX_TOP_COUNT"
                    | "APPROX_TOP_SUM"
                    // Statistical aggregate functions
                    | "CORR"
                    | "COVAR_POP"
                    | "COVAR_SAMP"
                    | "STDDEV"
                    | "STDDEV_POP"
                    | "STDDEV_SAMP"
                    | "VAR_POP"
                    | "VAR_SAMP"
                    | "VARIANCE"
                    // Geography aggregate functions
                    | "ST_CENTROID_AGG"
                    | "ST_UNION_AGG"
            ) {
                return true;
            }
        }
        current = parent.parent();
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::helpers::parse_sql;
    use rstest::rstest;
    use std::fs;

    #[rstest]
    #[case("invalid_group_by_column_not_in_clause.sql", 1)]
    #[case("invalid_group_by_multiple_violations.sql", 2)]
    #[case("invalid_group_by_in_subquery.sql", 1)]
    #[case("invalid_group_by_qualified_column.sql", 1)]
    #[case("invalid_group_by_mixed_qualified.sql", 1)]
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
    #[case("valid_group_by_all_aggregates.sql")]
    #[case("valid_group_by_approx_functions.sql")]
    #[case("valid_group_by_qualified_column.sql")]
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
    fn test_is_function_name() {
        let sql = "SELECT COUNT(col1) FROM table1";
        let tree = parse_sql(sql);

        // Check that function names are correctly identified
        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "identifier" {
                let text = get_node_text(&node, sql);
                if text == "COUNT" {
                    assert!(
                        is_function_name(&node),
                        "COUNT should be recognized as a function name"
                    );
                } else if text == "col1" {
                    assert!(
                        !is_function_name(&node),
                        "col1 should not be recognized as a function name"
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
