use std::collections::HashSet;
use std::sync::LazyLock;

use crate::ast::NodeRef;
use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::helpers::{find_child_of_kind, get_node_text, is_function_name, one_based_start};
use crate::rules::rule::Rule;

const RULE_ID: &str = "invalid_group_by";

/// BigQuery aggregate functions (uppercase, matched case-insensitively).
///
/// Reference: <https://cloud.google.com/bigquery/docs/reference/standard-sql/aggregate_functions>
static AGGREGATE_FUNCTIONS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        // Standard aggregate functions
        "ANY_VALUE",
        "ARRAY_AGG",
        "ARRAY_CONCAT_AGG",
        "AVG",
        "BIT_AND",
        "BIT_OR",
        "BIT_XOR",
        "COUNT",
        "COUNTIF",
        "GROUPING",
        "LOGICAL_AND",
        "LOGICAL_OR",
        "MAX",
        "MAX_BY",
        "MIN",
        "MIN_BY",
        "STRING_AGG",
        "SUM",
        // Approximate aggregate functions
        "APPROX_COUNT_DISTINCT",
        "APPROX_QUANTILES",
        "APPROX_TOP_COUNT",
        "APPROX_TOP_SUM",
        // Statistical aggregate functions
        "CORR",
        "COVAR_POP",
        "COVAR_SAMP",
        "STDDEV",
        "STDDEV_POP",
        "STDDEV_SAMP",
        "VAR_POP",
        "VAR_SAMP",
        "VARIANCE",
        // Geography aggregate functions
        "ST_CENTROID_AGG",
        "ST_UNION_AGG",
    ]
    .into_iter()
    .collect()
});

/// Flags SELECT columns that are neither grouped nor aggregated, which BigQuery
/// rejects at runtime.
pub struct InvalidGroupBy;

impl Rule for InvalidGroupBy {
    fn id(&self) -> &'static str {
        RULE_ID
    }

    fn check_node(&self, node: NodeRef<'_>, sql: &str, diagnostics: &mut Vec<Diagnostic>) {
        if node.kind() == "select"
            && let Some(diags) = check_select(&node, sql)
        {
            diagnostics.extend(diags);
        }
    }
}

fn check_select(node: &NodeRef<'_>, sql: &str) -> Option<Vec<Diagnostic>> {
    let group_by_columns = extract_group_by_columns(node, sql)?;

    let select_list = find_child_of_kind(node, "select_list")?;

    let mut diagnostics = Vec::new();
    for child in select_list.named_children() {
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

fn extract_group_by_columns(select_node: &NodeRef<'_>, sql: &str) -> Option<HashSet<String>> {
    let group_by_node = find_child_of_kind(select_node, "group_by_clause")?;
    let mut columns = HashSet::new();

    for node in group_by_node.pre_order() {
        if node.kind() == "identifier" {
            let text = get_node_text(&node, sql);
            columns.insert(text.to_string());
        }
    }

    Some(columns)
}

fn check_select_expression(
    expr_node: &NodeRef<'_>,
    sql: &str,
    group_by_columns: &HashSet<String>,
) -> Option<Diagnostic> {
    // Check if this expression contains an identifier that's not in an aggregate function
    for node in expr_node.pre_order() {
        if node.kind() == "identifier"
            && !is_alias(&node)
            && !is_function_name(&node)
            && !is_in_aggregate_function(&node, sql)
        {
            let field_text = get_node_text(&node, sql);

            // Check if the identifier is in GROUP BY
            if !group_by_columns.contains(field_text) {
                let (row, col) = one_based_start(&node);
                return Some(Diagnostic::new(
                    RULE_ID,
                    Severity::Error,
                    row,
                    col,
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

fn is_alias(node: &NodeRef<'_>) -> bool {
    // Check if this identifier is part of an as_alias
    node.parent()
        .is_some_and(|parent| parent.kind() == "as_alias")
}

fn is_in_aggregate_function(node: &NodeRef<'_>, sql: &str) -> bool {
    let mut current = node.parent();

    while let Some(parent) = current {
        if parent.kind() == "function_call"
            && let Some(func_node) = parent.child_by_field_name("function")
        {
            let func_name = get_node_text(&func_node, sql);

            if AGGREGATE_FUNCTIONS.contains(func_name.to_uppercase().as_str()) {
                return true;
            }
        }
        current = parent.parent();
    }

    false
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
    use crate::rules::helpers::{parse_sql, run_rule};
    use rstest::rstest;

    #[rstest]
    // col2 is not in GROUP BY and not in an aggregate function
    #[case(
        "\
SELECT col1, col2, COUNT(*) as cnt
FROM my_table
GROUP BY col1;
",
        1
    )]
    // Multiple columns not in GROUP BY
    #[case(
        "\
SELECT col1, col2, col3, COUNT(*) as cnt
FROM my_table
GROUP BY col1;
",
        2
    )]
    // Invalid GROUP BY in subquery should be detected
    #[case(
        "\
SELECT *
FROM (
  SELECT col1, col2, COUNT(*) as cnt
  FROM my_table
  GROUP BY col1
) sub;
",
        1
    )]
    // Qualified column name not in GROUP BY
    #[case(
        "\
SELECT t.col1, t.col2, COUNT(*) as cnt
FROM my_table t
GROUP BY t.col1;
",
        1
    )]
    // Mix of qualified and non-qualified columns; t1.col3 is not in GROUP BY
    #[case(
        "\
SELECT t1.col1, col2, t1.col3, COUNT(*) as cnt
FROM my_table t1
GROUP BY t1.col1, col2;
",
        1
    )]
    fn test_invalid_group_by(#[case] sql: &str, #[case] expected_count: usize) {
        let diagnostics = run_rule(&InvalidGroupBy, sql);
        assert_eq!(
            diagnostics.len(),
            expected_count,
            "Expected {} diagnostic(s), got {}",
            expected_count,
            diagnostics.len()
        );
    }

    #[rstest]
    // All non-aggregated columns are in GROUP BY, multiple GROUP BY columns,
    // and a query without GROUP BY (no violation).
    #[case(
        "\
SELECT col1, COUNT(col2) as cnt, SUM(col3) as total
FROM my_table
GROUP BY col1;

SELECT col1, col2, MAX(col3) as max_val
FROM my_table
GROUP BY col1, col2;

SELECT col1, col2, col3
FROM my_table;
"
    )]
    // Mixed case aggregate functions should work
    #[case(
        "\
SELECT col1, Count(col2) as cnt, SuM(col3) as total, mAx(col4) as max_val
FROM my_table
GROUP BY col1;
"
    )]
    // All BigQuery aggregate functions
    #[case(
        "\
SELECT
  col1,
  ANY_VALUE(col2) as any_val,
  ARRAY_AGG(col3) as arr,
  ARRAY_CONCAT_AGG(col4) as arr_concat,
  AVG(col5) as avg_val,
  BIT_AND(col6) as bit_and_val,
  BIT_OR(col7) as bit_or_val,
  BIT_XOR(col8) as bit_xor_val,
  COUNT(col9) as cnt,
  COUNTIF(col10 > 0) as cnt_if,
  LOGICAL_AND(col11) as logical_and_val,
  LOGICAL_OR(col12) as logical_or_val,
  MAX(col13) as max_val,
  MAX_BY(col14, col15) as max_by_val,
  MIN(col16) as min_val,
  MIN_BY(col17, col18) as min_by_val,
  STRING_AGG(col19) as str_agg,
  SUM(col20) as sum_val,
  APPROX_COUNT_DISTINCT(col21) as approx_cnt,
  APPROX_QUANTILES(col22, 4) as approx_quant,
  APPROX_TOP_COUNT(col23, 10) as approx_top_cnt,
  APPROX_TOP_SUM(col24, col25, 10) as approx_top_sum,
  CORR(col26, col27) as corr_val,
  COVAR_POP(col28, col29) as covar_pop_val,
  COVAR_SAMP(col30, col31) as covar_samp_val,
  STDDEV(col32) as stddev_val,
  STDDEV_POP(col33) as stddev_pop_val,
  STDDEV_SAMP(col34) as stddev_samp_val,
  VAR_POP(col35) as var_pop_val,
  VAR_SAMP(col36) as var_samp_val,
  VARIANCE(col37) as variance_val
FROM my_table
GROUP BY col1;
"
    )]
    // Approximate aggregate functions specifically
    #[case(
        "\
SELECT
  user_id,
  APPROX_COUNT_DISTINCT(product_id) as unique_products,
  APPROX_QUANTILES(price, 4) as price_quartiles,
  APPROX_TOP_COUNT(category, 5) as top_categories,
  APPROX_TOP_SUM(amount, item_name, 10) as top_items_by_amount
FROM sales_table
GROUP BY user_id;
"
    )]
    // Qualified column names correctly used with GROUP BY, plus a join
    #[case(
        "\
SELECT t.col1, t.col2, COUNT(t.col3) as cnt
FROM my_table t
GROUP BY t.col1, t.col2;

SELECT t1.user_id, t2.category, COUNT(*) as cnt
FROM users t1
JOIN orders t2 ON t1.id = t2.user_id
GROUP BY t1.user_id, t2.category;
"
    )]
    fn test_valid_group_by(#[case] sql: &str) {
        let diagnostics = run_rule(&InvalidGroupBy, sql);
        assert!(
            diagnostics.is_empty(),
            "Expected no diagnostics for valid GROUP BY, got {}",
            diagnostics.len()
        );
    }

    #[test]
    fn test_is_alias() {
        let sql = "SELECT col1 as alias1 FROM table1";
        let ast = parse_sql(sql);

        // Find the identifier "alias1"
        for node in ast.pre_order() {
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
        let ast = parse_sql(sql);

        // Check that function names are correctly identified
        for node in ast.pre_order() {
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
        let ast = parse_sql(sql);

        for node in ast.pre_order() {
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
