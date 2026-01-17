use tree_sitter::Node;

use crate::rules::unused_column_in_cte::{context::AnalysisContext, utils, visitor::NodeVisitor};

/// Visitor for processing QUALIFY clauses
/// QUALIFY is a BigQuery-specific clause for filtering window function results
pub struct QualifyVisitor;

impl NodeVisitor for QualifyVisitor {
    fn visit(&self, node: &Node, context: &mut AnalysisContext) {
        // Look for QUALIFY clause
        // BigQuery's QUALIFY is used after SELECT to filter window function results
        if node.kind() != "qualify_clause" {
            return;
        }

        let sql = context.sql();

        // Get the FROM clause to know which tables are available
        // We need to find the parent SELECT to get the FROM clause
        let select_node = find_parent_select(node);
        if select_node.is_none() {
            return;
        }

        let select_node = select_node.unwrap();
        let from_node = find_from_clause(&select_node);
        let (tables, alias_map) = utils::extract_table(from_node, sql);

        // Extract all field/identifier references from the QUALIFY clause
        utils::extract_and_mark_fields(node, sql, &tables, &alias_map, context);
    }
}

/// Find the parent SELECT node
fn find_parent_select<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "select" {
            return Some(parent);
        }
        current = parent.parent();
    }
    None
}

/// Find the FROM clause in a SELECT node
fn find_from_clause<'a>(select_node: &'a Node<'a>) -> Option<Node<'a>> {
    select_node
        .named_children(&mut select_node.walk())
        .find(|child| child.kind() == "from_clause")
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
    fn test_qualify_visitor() {
        let sql = "WITH cte1 AS (SELECT col1, col2, unused FROM table1) \
                   SELECT col2 FROM cte1 QUALIFY row_number() over (partition by col1) = 1";
        let tree = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let select_visitor = SelectVisitor::new();
        let qualify_visitor = QualifyVisitor;

        for node in traverse(tree.root_node().walk(), Order::Pre) {
            cte_visitor.visit(&node, &mut context);
            select_visitor.visit(&node, &mut context);
            qualify_visitor.visit(&node, &mut context);
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
