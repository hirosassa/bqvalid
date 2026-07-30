use crate::ast::NodeRef;

use crate::rules::helpers::find_parent_select;
use crate::rules::unused_column_in_cte::{context::AnalysisContext, utils, visitor::NodeVisitor};

/// Visitor for processing QUALIFY clauses
/// QUALIFY is a BigQuery-specific clause for filtering window function results
pub struct QualifyVisitor;

impl NodeVisitor for QualifyVisitor {
    fn visit(&self, node: NodeRef<'_>, context: &mut AnalysisContext) {
        // Look for QUALIFY clause
        // BigQuery's QUALIFY is used after SELECT to filter window function results
        if node.kind() != "qualify_clause" {
            return;
        }

        let sql = context.sql();

        let Some(select_node) = find_parent_select(&node) else {
            return;
        };

        let from_node = select_node
            .named_children()
            .into_iter()
            .find(|child| child.kind() == "from_clause");
        let (tables, alias_map) = utils::extract_table(from_node, sql);
        let resolver = utils::TableResolver::new(&tables, &alias_map, &context.cte_columns);

        // Extract all field/identifier references from the QUALIFY clause
        utils::extract_and_mark_fields(&node, sql, &resolver, context);
    }
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

    use crate::rules::unused_column_in_cte::visitors::{CteVisitor, SelectVisitor};

    #[test]
    fn test_qualify_visitor() {
        let sql = "WITH cte1 AS (SELECT col1, col2, unused FROM table1) \
                   SELECT col2 FROM cte1 QUALIFY row_number() over (partition by col1) = 1";
        let ast = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let select_visitor = SelectVisitor::new();
        let qualify_visitor = QualifyVisitor;

        for node in ast.pre_order() {
            cte_visitor.visit(node, &mut context);
            select_visitor.visit(node, &mut context);
            qualify_visitor.visit(node, &mut context);
        }

        // col1 should be marked as used (QUALIFY clause)
        assert!(context.graph.is_column_used("cte1", "col1"));
        // col2 should be marked as used (final SELECT)
        assert!(context.graph.is_column_used("cte1", "col2"));

        // unused should be the only unused column
        let unused = context.collect_unused();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].column_name, "unused");
    }
}
