use tree_sitter::Node;

use crate::rules::unused_column_in_cte::{context::AnalysisContext, visitor::NodeVisitor};

/// Visitor for processing SELECT DISTINCT
/// When DISTINCT is used, all columns affect the deduplication logic
/// Note: Currently not used - kept for potential future use
#[allow(dead_code)]
pub struct DistinctVisitor;

impl NodeVisitor for DistinctVisitor {
    fn visit(&self, node: &Node, context: &mut AnalysisContext) {
        if node.kind() != "select" {
            return;
        }

        let sql = context.sql();

        // Check if this SELECT has DISTINCT
        let has_distinct = node.children(&mut node.walk()).any(|child| {
            let text = child.utf8_text(sql.as_bytes()).unwrap_or("");
            text.eq_ignore_ascii_case("distinct")
        });

        if !has_distinct {
            return;
        }

        // Find which CTE this SELECT belongs to
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "cte" {
                // Get CTE name
                let cte_name = parent
                    .child_by_field_name("alias_name")
                    .and_then(|n| n.utf8_text(sql.as_bytes()).ok())
                    .unwrap_or("");

                // Mark all columns in this CTE as used
                // Clone column names first to avoid borrow checker issues
                let col_names: Vec<String> = context
                    .get_cte_columns(cte_name)
                    .map(|cols| cols.iter().map(|c| c.column_name.clone()).collect())
                    .unwrap_or_default();

                for col_name in col_names {
                    context.mark_used(cte_name, &col_name);
                }
                return;
            }
            current = parent.parent();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;
    use tree_sitter_traversal::{Order, traverse};

    use crate::rules::unused_column_in_cte::visitors::CteVisitor;

    fn parse_sql(sql: &str) -> tree_sitter::Tree {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();
        parser.parse(sql, None).unwrap()
    }

    #[test]
    fn test_distinct_visitor() {
        let sql = "WITH cte1 AS (SELECT DISTINCT col1, col2, col3 FROM table1) \
                   SELECT col1 FROM cte1";
        let tree = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let distinct_visitor = DistinctVisitor;

        for node in traverse(tree.root_node().walk(), Order::Pre) {
            cte_visitor.visit(&node, &mut context);
            distinct_visitor.visit(&node, &mut context);
        }

        // All columns should be marked as used (DISTINCT)
        assert!(context.graph.is_column_used("cte1", "col1"));
        assert!(context.graph.is_column_used("cte1", "col2"));
        assert!(context.graph.is_column_used("cte1", "col3"));

        let unused = context.collect_unused();
        assert_eq!(unused.len(), 0);
    }
}
