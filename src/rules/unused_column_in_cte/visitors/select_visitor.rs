use std::collections::VecDeque;
use tree_sitter::Node;

use crate::rules::unused_column_in_cte::{
    context::AnalysisContext, models::ColumnInfo, utils, visitor::NodeVisitor,
};

/// Visitor for processing SELECT statements and marking column usage
pub struct SelectVisitor;

impl SelectVisitor {
    pub const fn new() -> Self {
        Self
    }

    /// Check if we're in the final SELECT (not inside a CTE)
    fn is_final_select(&self, node: &Node) -> bool {
        // Check if this SELECT is inside a CTE
        let mut current = node.parent();
        while let Some(parent) = current {
            if parent.kind() == "cte" {
                return false; // Inside a CTE
            }
            current = parent.parent();
        }
        true // Not inside a CTE, so it's the final SELECT
    }
}

impl NodeVisitor for SelectVisitor {
    fn visit(&self, node: &Node, context: &mut AnalysisContext) {
        if node.kind() == "select" {
            let sql = context.sql();
            let is_final_select = self.is_final_select(node);

            // Find the select_list within the SELECT
            if let Some(select_list) = node
                .named_children(&mut node.walk())
                .find(|child| child.kind() == "select_list")
            {
                if is_final_select {
                    // For final SELECT: extract and mark columns through dependency tracing
                    let final_columns = extract_final_select_columns(&select_list, sql, context);
                    mark_used_columns(context, final_columns);
                } else {
                    // For CTE SELECT: extract column references from expressions (e.g., function arguments)
                    extract_and_mark_expression_columns(&select_list, node, sql, context);
                }
            }
        }
    }
}

impl Default for SelectVisitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract column references from CTE SELECT expressions and mark them as used
/// This handles columns used in function arguments, window functions, etc.
fn extract_and_mark_expression_columns(
    select_list: &Node,
    select_node: &Node,
    sql: &str,
    context: &mut AnalysisContext,
) {
    // First, determine which CTE this SELECT belongs to
    let cte_name = find_parent_cte_name(select_node, sql);
    if cte_name.is_empty() {
        return;
    }

    // Get the FROM clause to know which tables are available
    let from = select_list.next_named_sibling();
    let (tables, alias_map) = utils::extract_table(from, sql);

    // Process each select_expression
    for child in select_list.children(&mut select_list.walk()) {
        if child.kind() == "select_expression" {
            // Extract all field references from the expression (including nested ones in function calls)
            extract_field_references_from_expression(&child, sql, &tables, &alias_map, context);
        }
    }
}

/// Find the CTE name that contains this SELECT node
fn find_parent_cte_name(select_node: &Node, sql: &str) -> String {
    let mut current = select_node.parent();
    while let Some(parent) = current {
        if parent.kind() == "cte" {
            return parent
                .child_by_field_name("alias_name")
                .and_then(|n| n.utf8_text(sql.as_bytes()).ok())
                .unwrap_or("")
                .to_string();
        }
        current = parent.parent();
    }
    String::new()
}

/// Recursively extract all field references from an expression
/// This handles: direct references, function arguments, window functions, OVER clauses, etc.
fn extract_field_references_from_expression(
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

        // Skip if this is a function name
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
        extract_field_references_from_expression(&child, sql, tables, alias_map, context);
    }
}

/// Extract columns from final SELECT
/// This extracts both simple column references and nested field references (e.g., in functions)
fn extract_final_select_columns(
    select_list: &Node,
    sql: &str,
    context: &AnalysisContext,
) -> Vec<ColumnInfo> {
    let mut columns = Vec::new();
    let from = select_list.next_named_sibling();
    let (tables, alias_map) = utils::extract_table(from, sql);

    for child in select_list.children(&mut select_list.walk()) {
        if child.kind() == "select_expression" {
            // Strategy: First try to get the direct field reference,
            // then recursively find all field nodes (for function arguments, etc.)

            // Check if this has a direct field child (simple column reference)
            let has_direct_field = child
                .children(&mut child.walk())
                .any(|c| c.kind() == "field");

            if !has_direct_field {
                // Fallback: treat the whole expression text as a column reference
                // This handles cases where tree-sitter doesn't create a 'field' node
                let column_text = child.utf8_text(sql.as_bytes()).unwrap();
                let col_name = utils::extract_column_name(column_text);

                let table = utils::find_original_table(
                    column_text,
                    &tables,
                    &alias_map,
                    &context.cte_columns,
                );

                if !table.is_empty() {
                    columns.push(ColumnInfo::new(
                        Some(table),
                        col_name.to_string(),
                        None,
                        child.start_position().row,
                        child.start_position().column,
                    ));
                }
            }

            // Recurse to find any nested fields (after both branches)
            extract_all_fields_into_vec(
                &child,
                sql,
                &tables,
                &alias_map,
                &context.cte_columns,
                &mut columns,
            );
        } else if child.kind() == "select_all" {
            // SELECT * - expand to all columns from referenced tables
            for table in &tables {
                if let Some(cols) = context.get_cte_columns(table) {
                    for col in cols {
                        columns.push(ColumnInfo::new(
                            Some(table.clone()),
                            col.column_name.clone(),
                            None,
                            child.start_position().row,
                            child.start_position().column,
                        ));
                    }
                }
            }
        }
    }

    columns
}

/// Recursively extract all 'field' nodes from an AST subtree
fn extract_all_fields_into_vec(
    node: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &std::collections::HashMap<String, String>,
    cte_columns: &std::collections::HashMap<String, Vec<ColumnInfo>>,
    columns: &mut Vec<ColumnInfo>,
) {
    // Process current node if it's a field or identifier
    // Note: tree-sitter may use 'field' for qualified references (table.column)
    // and 'identifier' for simple column references
    if node.kind() == "field" || node.kind() == "identifier" {
        let field_text = node.utf8_text(sql.as_bytes()).unwrap_or("");
        let col_name = utils::extract_column_name(field_text);

        // Skip if this looks like a function name (parent is function_call with this as name)
        if let Some(parent) = node.parent()
            && parent.kind() == "function_call"
            && let Some(name_node) = parent.child_by_field_name("name")
            && name_node.id() == node.id()
        {
            // This is a function name, not a column reference
            return;
        }

        // Find which table this column belongs to
        let table = utils::find_original_table(field_text, tables, alias_map, cte_columns);

        if !table.is_empty() {
            columns.push(ColumnInfo::new(
                Some(table),
                col_name.to_string(),
                None,
                node.start_position().row,
                node.start_position().column,
            ));
        }
    }

    // Recursively process all children
    for child in node.children(&mut node.walk()) {
        extract_all_fields_into_vec(&child, sql, tables, alias_map, cte_columns, columns);
    }
}

/// Mark columns as used and trace dependencies
fn mark_used_columns(context: &mut AnalysisContext, final_columns: Vec<ColumnInfo>) {
    let mut queue: VecDeque<(String, String)> = VecDeque::new();

    // Add final select columns to queue
    for col in final_columns {
        if let Some(table_name) = col.table_name {
            queue.push_back((table_name, col.column_name));
        }
    }

    // Process queue until empty
    while let Some((table_name, column_name)) = queue.pop_front() {
        // Skip if already marked
        if context.graph.is_column_used(&table_name, &column_name) {
            continue;
        }

        // Mark as used
        context.mark_used(&table_name, &column_name);

        // Trace dependencies: find where this column comes from
        if let Some(cte_columns) = context.get_cte_columns(&table_name) {
            for col_info in cte_columns {
                let col_base_name = utils::extract_column_name(&col_info.column_name);
                let search_base_name = utils::extract_column_name(&column_name);

                if col_base_name == search_base_name || col_info.column_name == column_name {
                    // Found the column definition, trace its source
                    if let Some(source_table) = &col_info.table_name {
                        let actual_source_table = utils::extract_table_name(source_table);

                        // Only trace back if source_table is also a CTE
                        if context.has_cte(actual_source_table) {
                            let search_column_name = col_info
                                .original_column_name
                                .as_ref()
                                .unwrap_or(&col_info.column_name)
                                .clone();
                            queue.push_back((actual_source_table.to_string(), search_column_name));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;
    use tree_sitter_traversal::{Order, traverse};

    fn parse_sql(sql: &str) -> tree_sitter::Tree {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();
        parser.parse(sql, None).unwrap()
    }

    #[test]
    fn test_select_visitor() {
        use crate::rules::unused_column_in_cte::visitors::CteVisitor;

        let sql = "WITH cte1 AS (SELECT col1, col2, col3 FROM table1) SELECT col1, col2 FROM cte1";
        let tree = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let select_visitor = SelectVisitor::new();

        // First pass: collect CTEs
        for node in traverse(tree.root_node().walk(), Order::Pre) {
            cte_visitor.visit(&node, &mut context);
        }

        // Second pass: mark usage
        for node in traverse(tree.root_node().walk(), Order::Pre) {
            select_visitor.visit(&node, &mut context);
        }

        // Check that col1 and col2 are marked as used
        assert!(context.graph.is_column_used("cte1", "col1"));
        assert!(context.graph.is_column_used("cte1", "col2"));

        // col3 should be unused
        let unused = context.collect_unused();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].column_name, "col3");
    }
}
