use tree_sitter::Node;

use crate::rules::unused_column_in_cte::{context::AnalysisContext, utils, visitor::NodeVisitor};

/// Visitor for processing PIVOT clauses
/// PIVOT is a BigQuery feature for transforming rows into columns
pub struct PivotVisitor;

impl NodeVisitor for PivotVisitor {
    fn visit(&self, node: &Node, context: &mut AnalysisContext) {
        // Look for PIVOT operator
        if node.kind() != "pivot_operator" {
            return;
        }

        let sql = context.sql();

        // Get the FROM clause to know which tables are available
        // Find the parent FROM clause
        let (tables, alias_map) = find_tables_for_pivot(node, sql);

        // Extract all field/identifier references from the PIVOT clause
        utils::extract_and_mark_fields(node, sql, &tables, &alias_map, context);
    }
}

/// Find tables available in the context of a PIVOT operator
fn find_tables_for_pivot(
    pivot_node: &Node,
    sql: &str,
) -> (Vec<String>, std::collections::HashMap<String, String>) {
    // Walk up to find the FROM clause or table reference
    let mut current = pivot_node.parent();
    while let Some(parent) = current {
        // Check if this is a from_clause
        if parent.kind() == "from_clause" {
            return utils::extract_table(Some(parent), sql);
        }
        // Check if this is inside a SELECT that has a FROM clause
        if parent.kind() == "select" {
            for child in parent.named_children(&mut parent.walk()) {
                if child.kind() == "from_clause" {
                    return utils::extract_table(Some(child), sql);
                }
            }
        }
        current = parent.parent();
    }
    (Vec::new(), std::collections::HashMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;
    use tree_sitter_traversal::{Order, traverse};

    use crate::rules::unused_column_in_cte::visitors::{CteVisitor, SelectVisitor};

    fn parse_sql(sql: &str) -> tree_sitter::Tree {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();
        parser.parse(sql, None).unwrap()
    }

    #[test]
    fn test_pivot_visitor() {
        let sql = "WITH raw_data AS (SELECT category, month, value, unused FROM table1) \
                   SELECT category FROM raw_data PIVOT(sum(value) for month in ('Jan' as jan))";
        let tree = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let select_visitor = SelectVisitor::new();
        let pivot_visitor = PivotVisitor;

        for node in traverse(tree.root_node().walk(), Order::Pre) {
            cte_visitor.visit(&node, &mut context);
            select_visitor.visit(&node, &mut context);
            pivot_visitor.visit(&node, &mut context);
        }

        // value and month should be marked as used (PIVOT clause)
        assert!(context.graph.is_column_used("raw_data", "value"));
        assert!(context.graph.is_column_used("raw_data", "month"));
        // category should be marked as used (final SELECT)
        assert!(context.graph.is_column_used("raw_data", "category"));

        // unused should be the only unused column
        let unused = context.collect_unused();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].column_name, "unused");
    }
}
