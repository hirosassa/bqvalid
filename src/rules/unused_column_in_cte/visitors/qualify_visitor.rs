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
        extract_and_mark_fields(node, sql, &tables, &alias_map, context);
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

/// Recursively extract all field/identifier references and mark them as used
fn extract_and_mark_fields(
    node: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &std::collections::HashMap<String, String>,
    context: &mut AnalysisContext,
) {
    // Process current node if it's a field or identifier
    if node.kind() == "field" || node.kind() == "identifier" {
        let field_text = node.utf8_text(sql.as_bytes()).unwrap_or("");
        let col_name = utils::extract_column_name(field_text);

        // Skip function names
        if let Some(parent) = node.parent()
            && parent.kind() == "function_call"
            && let Some(name_node) = parent.child_by_field_name("name")
            && name_node.id() == node.id()
        {
            // This is a function name, skip it
            return;
        }

        // Find which table this column belongs to
        let table = utils::find_original_table(field_text, tables, alias_map, &context.cte_columns);

        if !table.is_empty() {
            context.mark_used(&table, col_name);
        }
    }

    // Recursively process children
    for child in node.children(&mut node.walk()) {
        extract_and_mark_fields(&child, sql, tables, alias_map, context);
    }
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
