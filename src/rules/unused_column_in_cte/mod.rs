// Visitor-based implementation modules
mod context;
mod graph;
mod models;
mod utils;
mod visitor;
mod visitors;

use tree_sitter::Tree;
use tree_sitter_traversal::{Order, traverse};

use crate::diagnostic::Diagnostic;

use context::AnalysisContext;
use visitor::NodeVisitor;
use visitors::{
    CteVisitor, PivotVisitor, QualifyVisitor, SelectStarVisitor, SelectVisitor, WhereVisitor,
};

pub fn check(tree: &Tree, sql: &str) -> Option<Vec<Diagnostic>> {
    let mut context = AnalysisContext::new(sql);

    let cte_visitor = CteVisitor;
    let select_star_visitor = SelectStarVisitor;
    let select_visitor = SelectVisitor::new();
    let where_visitor = WhereVisitor;
    let qualify_visitor = QualifyVisitor;
    let pivot_visitor = PivotVisitor;

    // Single-pass traversal with all visitors
    // Note: DistinctVisitor removed - DISTINCT doesn't make all CTE columns used,
    // only the columns in the SELECT clause are affected by DISTINCT
    for node in traverse(tree.root_node().walk(), Order::Pre) {
        cte_visitor.visit(&node, &mut context);
        select_star_visitor.visit(&node, &mut context);
        select_visitor.visit(&node, &mut context);
        where_visitor.visit(&node, &mut context);
        qualify_visitor.visit(&node, &mut context);
        pivot_visitor.visit(&node, &mut context);
    }

    let unused_columns = context.collect_unused();

    if unused_columns.is_empty() {
        None
    } else {
        Some(
            unused_columns
                .into_iter()
                .map(|col| {
                    Diagnostic::new(
                        col.row,
                        col.col,
                        format!("Unused column: {}", col.column_name),
                    )
                })
                .collect(),
        )
    }
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

    /// Integration tests using SQL files from sql/ directory
    /// Each test case specifies the SQL file name and expected unused column names
    #[rstest]
    #[case("unused_column_in_cte_simple.sql", vec!["unused_column1", "unused_column2"])]
    #[case("unused_column_in_cte_column_alias.sql", vec!["column2", "unused_column"])]
    #[case("unused_column_in_cte_function_argument.sql", vec!["unused_field"])]
    #[case("unused_column_in_cte_join_only.sql", vec!["unused_field"])]
    #[case("unused_column_in_cte_complex.sql", vec!["unused_field1", "unused_field2", "unused_amount_field", "unused_price_field", "another_unused"])]
    #[case("unused_column_in_cte_multiple_alias.sql", vec!["email", "unused_field1", "unused_field2"])]
    #[case("unused_column_in_cte_table_alias_without_as.sql", vec!["id", "name"])]
    #[case("unused_column_in_cte_where_with_qualified_table.sql", vec!["contract_type", "base_fee", "account_fee", "free_account_count"])]
    #[case("unused_column_in_cte_qualify.sql", vec!["unused_field"])]
    #[case("unused_column_in_cte_select_star_with_unused.sql", vec!["unused_field1", "unused_field2"])]
    #[case("unused_column_in_cte_select_star_multiple_joins.sql", vec!["id"])]
    fn test_integration_with_sql_files(#[case] filename: &str, #[case] expected_unused: Vec<&str>) {
        let sql_path = format!("sql/{}", filename);
        let sql =
            fs::read_to_string(&sql_path).unwrap_or_else(|_| panic!("Failed to read {}", sql_path));

        let tree = parse_sql(&sql);
        let result = check(&tree, &sql);

        if expected_unused.is_empty() {
            assert!(
                result.is_none(),
                "{}: Expected no unused columns, but found some",
                filename
            );
        } else {
            assert!(
                result.is_some(),
                "{}: Expected unused columns, but found none",
                filename
            );

            let diagnostics = result.unwrap();
            let mut found_columns: Vec<String> = diagnostics
                .iter()
                .map(|d| {
                    // Extract column name from "Unused column: <name>" message
                    d.message()
                        .strip_prefix("Unused column: ")
                        .unwrap_or(d.message())
                        .to_string()
                })
                .collect();

            found_columns.sort();
            let mut expected_sorted = expected_unused
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>();
            expected_sorted.sort();

            assert_eq!(
                found_columns, expected_sorted,
                "{}: Unused columns mismatch.\nExpected: {:?}\nFound: {:?}",
                filename, expected_sorted, found_columns
            );
        }
    }

    /// Test cases that should report no unused columns
    #[rstest]
    #[case("unused_column_in_cte_aggregate_in_final_select.sql")]
    #[case("unused_column_in_cte_join_where.sql")]
    #[case("unused_column_in_cte_table_alias.sql")]
    #[case("unused_column_in_cte_table_alias_join.sql")]
    #[case("unused_column_in_cte_unnest_in_from.sql")]
    #[case("unused_column_in_cte_pivot.sql")]
    #[case("unused_column_in_cte_select_star_chain.sql")]
    #[case("unused_column_in_cte_select_star_from_join.sql")]
    fn test_integration_no_unused_columns(#[case] filename: &str) {
        let sql_path = format!("sql/{}", filename);
        let sql =
            fs::read_to_string(&sql_path).unwrap_or_else(|_| panic!("Failed to read {}", sql_path));

        let tree = parse_sql(&sql);
        let result = check(&tree, &sql);

        assert!(
            result.is_none(),
            "{}: Expected no unused columns, but found: {:?}",
            filename,
            result.map(|d| d
                .iter()
                .map(|diag| diag.message().to_string())
                .collect::<Vec<_>>())
        );
    }
}
