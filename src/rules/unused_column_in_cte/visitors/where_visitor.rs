use std::collections::HashMap;

use crate::ast::NodeRef;
use crate::rules::helpers::{
    find_child_of_kind, find_parent_select, get_node_text, is_function_name,
};
use crate::rules::unused_column_in_cte::{
    context::AnalysisContext, models::ColumnInfo, utils, visitor::NodeVisitor,
};

/// Visitor for processing WHERE clauses, JOIN conditions, GROUP BY, HAVING,
/// ORDER BY, and JOIN ... USING.
pub struct WhereVisitor;

impl NodeVisitor for WhereVisitor {
    fn visit(&self, node: NodeRef<'_>, context: &mut AnalysisContext) {
        // Every column reference (`ASTPathExpression`) appearing in one of these
        // clauses is a use of that column. ORDER BY hangs off `ASTQuery` as a
        // sibling of the SELECT (not a child of it), so table resolution falls
        // back to that query's SELECT — see `extract_tables_from_parent`.
        match node.kind() {
            "ASTOnClause" | "ASTWhereClause" | "ASTGroupBy" | "ASTHaving" | "ASTOrderBy" => {
                process_condition_node(&node, context);
            }
            // Process UNNEST functions in FROM clause
            "ASTFromClause" => process_unnest_in_from(&node, context),
            // JOIN ... USING(col) references `col` on every joined table.
            "ASTUsingClause" => process_using_clause(&node, context),
            _ => {}
        }
    }
}

/// Process a condition node and mark column references as used
fn process_condition_node(node: &NodeRef<'_>, context: &mut AnalysisContext) {
    let sql = context.sql();
    let (tables, alias_map) = extract_tables_from_parent(node, sql);
    let resolver = utils::TableResolver::new(&tables, &alias_map, &context.cte_columns);

    // Extract all column references from the condition
    let mut col_refs = Vec::new();
    extract_columns_from_condition(node, sql, &resolver, &mut col_refs);

    // Mark each column reference as used
    for col_ref in col_refs {
        if let Some(table_name) = col_ref.table_name {
            let col_name = utils::extract_column_name(&col_ref.column_name);
            context.mark_used(&table_name, col_name);
        }
    }
}

/// Extract tables from the SELECT that owns this clause.
fn extract_tables_from_parent(
    node: &NodeRef<'_>,
    sql: &str,
) -> (Vec<String>, HashMap<String, String>) {
    if let Some(select_node) = enclosing_select(node) {
        let from_node = find_child_of_kind(&select_node, "ASTFromClause");
        return utils::extract_table(from_node, sql);
    }
    (Vec::new(), HashMap::new())
}

/// Find the SELECT that a clause belongs to.
///
/// Most clauses (WHERE, GROUP BY, HAVING, JOIN ON/USING) are descendants of the
/// `ASTSelect`, so the nearest-ancestor SELECT is correct. ORDER BY (and LIMIT)
/// instead hang off the enclosing `ASTQuery` as a direct sibling of the SELECT,
/// so no SELECT ancestor exists; in that case take the SELECT child of that
/// `ASTQuery`. When the query's body is a set operation (e.g. `... UNION ALL
/// ... ORDER BY x`) there is no single owning SELECT and this returns `None`, so
/// the caller degrades to an empty table set rather than mis-resolving.
fn enclosing_select<'a>(node: &NodeRef<'a>) -> Option<NodeRef<'a>> {
    find_parent_select(node).or_else(|| {
        node.parent()
            .filter(|parent| parent.kind() == "ASTQuery")
            .and_then(|query| find_child_of_kind(&query, "ASTSelect"))
    })
}

/// Mark the columns named in a JOIN ... USING(...) clause as used.
///
/// A USING column is unqualified and refers to the same-named column on *every*
/// joined table, so it must be marked used on all in-scope tables that define
/// it (resolving it to a single owner would leave the other side's column
/// falsely flagged as unused). On googlesql the column names inside USING are
/// bare `ASTIdentifier`s rather than `ASTPathExpression`s.
fn process_using_clause(node: &NodeRef<'_>, context: &mut AnalysisContext) {
    let sql = context.sql();
    let (tables, _alias_map) = extract_tables_from_parent(node, sql);

    for child in node.pre_order() {
        if child.kind() != "ASTIdentifier" {
            continue;
        }
        let col_name = get_node_text(&child, sql);
        let owners: Vec<String> = tables
            .iter()
            .filter(|table| {
                context
                    .get_cte_columns(table)
                    .is_some_and(|cols| cols.iter().any(|c| c.column_name == col_name))
            })
            .cloned()
            .collect();
        for owner in owners {
            context.mark_used(&owner, col_name);
        }
    }
}

/// Process UNNEST functions in FROM clause and mark their column arguments as used
fn process_unnest_in_from(from_node: &NodeRef<'_>, context: &mut AnalysisContext) {
    let sql = context.sql();
    let (tables, alias_map) = utils::extract_table(Some(*from_node), sql);
    let resolver = utils::TableResolver::new(&tables, &alias_map, &context.cte_columns);

    // Collect column references first
    let mut col_refs = Vec::new();

    // Find all UNNEST clauses in FROM clause (BigQuery-specific syntax)
    for child in from_node.pre_order() {
        if child.kind() == "ASTUnnestExpression" {
            // Extract identifiers from UNNEST - these are the column references
            for unnest_child in child.pre_order() {
                if unnest_child.kind() == "ASTPathExpression" {
                    let column_text = get_node_text(&unnest_child, sql);
                    let table = resolver.resolve_qualified_or(column_text);

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
    node: &NodeRef<'_>,
    sql: &str,
    resolver: &utils::TableResolver,
    columns: &mut Vec<ColumnInfo>,
) {
    // Traverse the condition tree to find all column references
    for child in node.pre_order() {
        if child.kind() == "ASTPathExpression" {
            // Skip function names
            if is_function_name(&child) {
                continue;
            }

            let column_text = get_node_text(&child, sql).to_string();
            let table = resolver.resolve_qualified_or(&column_text);

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
    fn test_where_visitor() {
        let sql = "WITH cte1 AS (SELECT col1, col2, col3 FROM table1) \
                   SELECT col1 FROM cte1 WHERE col2 > 10";
        let ast = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let select_visitor = SelectVisitor::new();
        let where_visitor = WhereVisitor;

        // Single pass with all visitors
        for node in ast.pre_order() {
            cte_visitor.visit(node, &mut context);
            select_visitor.visit(node, &mut context);
            where_visitor.visit(node, &mut context);
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
        let ast = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let cte_visitor = CteVisitor;
        let select_visitor = SelectVisitor::new();
        let where_visitor = WhereVisitor;

        for node in ast.pre_order() {
            cte_visitor.visit(node, &mut context);
            select_visitor.visit(node, &mut context);
            where_visitor.visit(node, &mut context);
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
