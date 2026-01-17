use std::collections::HashMap;
use tree_sitter::Node;
use tree_sitter_traversal::{Order, traverse};

use crate::rules::unused_column_in_cte::{
    context::AnalysisContext, models::ColumnInfo, utils, visitor::NodeVisitor,
};

/// Visitor for processing WHERE clauses, JOIN conditions, and GROUP BY
pub struct WhereVisitor;

impl NodeVisitor for WhereVisitor {
    fn visit(&self, node: &Node, context: &mut AnalysisContext) {
        // Process JOIN conditions, WHERE clauses, and GROUP BY
        if matches!(
            node.kind(),
            "join_condition" | "where_clause" | "group_by_clause"
        ) {
            process_condition_node(node, context);
        } else if node.kind() == "from_clause" {
            // Process UNNEST functions in FROM clause
            process_unnest_in_from(node, context);
        }
    }
}

/// Process a condition node and mark column references as used
fn process_condition_node(node: &Node, context: &mut AnalysisContext) {
    let sql = context.sql();
    let (tables, alias_map) = extract_tables_from_parent(node, sql);

    // Extract all column references from the condition
    let mut col_refs = Vec::new();
    extract_columns_from_condition(
        node,
        sql,
        &tables,
        &alias_map,
        &context.cte_columns,
        &mut col_refs,
    );

    // Mark each column reference as used
    for col_ref in col_refs {
        if let Some(table_name) = col_ref.table_name {
            let col_name = utils::extract_column_name(&col_ref.column_name);
            context.mark_used(&table_name, col_name);
        }
    }
}

/// Extract tables from the parent SELECT node
fn extract_tables_from_parent(node: &Node, sql: &str) -> (Vec<String>, HashMap<String, String>) {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "select" {
            // Find the from_clause child of this SELECT
            for child in parent.named_children(&mut parent.walk()) {
                if child.kind() == "from_clause" {
                    return utils::extract_table(Some(child), sql);
                }
            }
        }
        current = parent.parent();
    }
    (Vec::new(), HashMap::new())
}

/// Process UNNEST functions in FROM clause and mark their column arguments as used
fn process_unnest_in_from(from_node: &Node, context: &mut AnalysisContext) {
    let sql = context.sql();
    let (tables, alias_map) = utils::extract_table(Some(*from_node), sql);

    // Collect column references first
    let mut col_refs = Vec::new();

    // Find all UNNEST clauses in FROM clause (BigQuery-specific syntax)
    for child in traverse(from_node.walk(), Order::Pre) {
        if child.kind() == "unnest_operator" || child.kind() == "unnest_clause" {
            // Extract identifiers from UNNEST - these are the column references
            for unnest_child in traverse(child.walk(), Order::Pre) {
                if unnest_child.kind() == "identifier" || unnest_child.kind() == "field" {
                    let column_text = unnest_child.utf8_text(sql.as_bytes()).unwrap();

                    // Resolve table name for this column
                    let table = if column_text.contains('.') {
                        // Qualified reference: table.column
                        let prefix = column_text.split('.').next().unwrap_or("");
                        alias_map
                            .get(prefix)
                            .cloned()
                            .unwrap_or_else(|| prefix.to_string())
                    } else {
                        // Unqualified reference: find which table it belongs to
                        utils::find_original_table(
                            column_text,
                            &tables,
                            &alias_map,
                            &context.cte_columns,
                        )
                    };

                    if !table.is_empty() {
                        col_refs.push(ColumnInfo::new(
                            Some(table),
                            column_text.to_string(),
                            None,
                            unnest_child.start_position().row,
                            unnest_child.start_position().column,
                        ));
                    }
                }
            }
        }
    }

    // Mark all collected columns as used
    for col_ref in col_refs {
        if let Some(table_name) = col_ref.table_name {
            let col_name = utils::extract_column_name(&col_ref.column_name);
            context.mark_used(&table_name, col_name);
        }
    }
}

/// Extract column references from a condition node
fn extract_columns_from_condition(
    node: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    columns: &mut Vec<ColumnInfo>,
) {
    // Traverse the condition tree to find all column references
    for child in traverse(node.walk(), Order::Pre) {
        if child.kind() == "field" || child.kind() == "identifier" {
            // Skip function names
            if utils::is_function_name(&child) {
                continue;
            }

            let column_text = child.utf8_text(sql.as_bytes()).unwrap().to_string();

            // Resolve table name for this column
            let table = if column_text.contains('.') {
                // Qualified reference: table.column
                let prefix = column_text.split('.').next().unwrap_or("");
                alias_map
                    .get(prefix)
                    .cloned()
                    .unwrap_or_else(|| prefix.to_string())
            } else {
                // Unqualified reference: find which table it belongs to
                utils::find_original_table(&column_text, tables, alias_map, cte_columns)
            };

            if !table.is_empty() {
                columns.push(ColumnInfo::new(
                    Some(table),
                    column_text,
                    None,
                    child.start_position().row,
                    child.start_position().column,
                ));
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

    use crate::rules::unused_column_in_cte::visitors::{CteVisitor, SelectVisitor};

    fn parse_sql(sql: &str) -> tree_sitter::Tree {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();
        parser.parse(sql, None).unwrap()
    }

    #[test]
    fn test_where_visitor() {
        let sql = "WITH cte1 AS (SELECT col1, col2, col3 FROM table1) \
                   SELECT col1 FROM cte1 WHERE col2 > 10";
        let tree = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let select_visitor = SelectVisitor::new();
        let where_visitor = WhereVisitor;

        // Single pass with all visitors
        for node in traverse(tree.root_node().walk(), Order::Pre) {
            cte_visitor.visit(&node, &mut context);
            select_visitor.visit(&node, &mut context);
            where_visitor.visit(&node, &mut context);
        }

        // col1 should be marked as used (in SELECT)
        assert!(context.graph.is_column_used("cte1", "col1"));
        // col2 should be marked as used (in WHERE)
        assert!(context.graph.is_column_used("cte1", "col2"));

        // col3 should still be unused
        let unused = context.collect_unused();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].column_name, "col3");
    }

    #[test]
    fn test_join_condition() {
        let sql = "WITH cte1 AS (SELECT id, name, unused FROM t1), \
                        cte2 AS (SELECT id, value FROM t2) \
                   SELECT cte1.name, cte2.value FROM cte1 \
                   JOIN cte2 ON cte1.id = cte2.id";
        let tree = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let select_visitor = SelectVisitor::new();
        let where_visitor = WhereVisitor;

        for node in traverse(tree.root_node().walk(), Order::Pre) {
            cte_visitor.visit(&node, &mut context);
            select_visitor.visit(&node, &mut context);
            where_visitor.visit(&node, &mut context);
        }

        // name and value should be marked as used (in SELECT)
        assert!(context.graph.is_column_used("cte1", "name"));
        assert!(context.graph.is_column_used("cte2", "value"));
        // id columns should be marked as used (in JOIN condition)
        assert!(context.graph.is_column_used("cte1", "id"));
        assert!(context.graph.is_column_used("cte2", "id"));

        // unused should still be unused
        let unused = context.collect_unused();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].column_name, "unused");
    }
}
