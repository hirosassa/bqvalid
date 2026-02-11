use tree_sitter::Node;

use crate::rules::unused_column_in_cte::{context::AnalysisContext, utils, visitor::NodeVisitor};

/// Visitor for processing SELECT * in the final SELECT
/// This marks all columns from the referenced tables as used
pub struct SelectStarVisitor;

impl NodeVisitor for SelectStarVisitor {
    fn visit(&self, node: &Node, context: &mut AnalysisContext) {
        if node.kind() != "select_list" {
            return;
        }

        // Check if this select_list contains SELECT *
        let has_select_star = node
            .children(&mut node.walk())
            .any(|child| child.kind() == "select_all");

        if !has_select_star {
            return;
        }

        // Check if this is in the final SELECT (not in a CTE)
        let mut current = node.parent();
        let mut in_cte = false;
        while let Some(parent) = current {
            if parent.kind() == "cte" {
                in_cte = true;
                break;
            }
            current = parent.parent();
        }

        // Only process SELECT * in final SELECT, not in CTEs
        if !in_cte {
            mark_source_columns_as_used(node, context);
        }
    }
}

/// Mark all columns from source tables as used
fn mark_source_columns_as_used(select_list: &Node, context: &mut AnalysisContext) {
    let sql = context.sql();

    // Find the FROM clause
    let from = select_list.next_named_sibling();
    let (tables, _alias_map) = utils::extract_table(from, sql);

    // Mark all columns from these tables as used
    for table in &tables {
        // Clone column names first to avoid borrow checker issues
        let col_names: Vec<String> = context
            .get_cte_columns(table)
            .map(|cols| cols.iter().map(|c| c.column_name.clone()).collect())
            .unwrap_or_default();

        for col_name in col_names {
            context.mark_used(table, &col_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::helpers::parse_sql;
    use tree_sitter_traversal::{Order, traverse};

    use crate::rules::unused_column_in_cte::visitors::{CteVisitor, SelectVisitor};

    #[test]
    fn test_select_star_visitor() {
        let sql = "WITH cte1 AS (SELECT col1, col2, unused FROM table1) \
                   SELECT * FROM cte1";
        let tree = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let select_star_visitor = SelectStarVisitor;
        let select_visitor = SelectVisitor::new();

        for node in traverse(tree.root_node().walk(), Order::Pre) {
            cte_visitor.visit(&node, &mut context);
            select_star_visitor.visit(&node, &mut context);
            select_visitor.visit(&node, &mut context);
        }

        // All columns should be marked as used (final SELECT *)
        assert!(context.graph.is_column_used("cte1", "col1"));
        assert!(context.graph.is_column_used("cte1", "col2"));
        assert!(context.graph.is_column_used("cte1", "unused"));

        let unused = context.collect_unused();
        assert_eq!(unused.len(), 0); // All columns used
    }
}
