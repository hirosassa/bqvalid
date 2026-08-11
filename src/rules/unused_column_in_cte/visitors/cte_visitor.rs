use crate::ast::{NodeRef, Point};
use crate::rules::helpers::{find_child_of_kind, get_node_text};
use crate::rules::unused_column_in_cte::{
    context::AnalysisContext, models::ColumnInfo, utils, visitor::NodeVisitor,
};

/// Visitor for collecting CTE definitions
pub struct CteVisitor;

impl NodeVisitor for CteVisitor {
    fn visit(&self, node: NodeRef<'_>, context: &mut AnalysisContext) {
        // A CTE definition: `ASTAliasedQuery` on googlesql.
        if node.kind() != "ASTAliasedQuery" {
            return;
        }

        let sql = context.sql();
        let cte_name = utils::get_cte_name(&node, sql).to_string();

        // Find the SELECT (or the query wrapper around it) that defines this CTE.
        // googlesql wraps the SELECT in an `ASTQuery`.
        let query_node = node
            .named_children()
            .into_iter()
            .find(|child| matches!(child.kind(), "ASTSelect" | "ASTQuery"));

        if let Some(query) = query_node {
            let select_node = if query.kind() == "ASTQuery" {
                find_child_of_kind(&query, "ASTSelect")
            } else {
                Some(query)
            };

            if let Some(sel) = select_node {
                // Find the select_list within this SELECT
                if let Some(select_list) = find_child_of_kind(&sel, "ASTSelectList") {
                    let columns = extract_columns(&select_list, sql, &context.cte_columns);
                    context.add_cte(cte_name, columns);
                }
            }
        }
    }
}

/// Extract columns from a select_list node.
///
/// Callers always pass the CTE's `select_list`, so this handles that node
/// directly rather than recursing.
fn extract_columns(
    node: &NodeRef<'_>,
    sql: &str,
    cte_columns: &std::collections::HashMap<String, Vec<ColumnInfo>>,
) -> Vec<ColumnInfo> {
    let mut columns = Vec::new();

    if node.kind() != "ASTSelectList" {
        return columns;
    }

    let from = node.next_named_sibling();
    let (tables, alias_map) = utils::extract_table(from, sql);
    let resolver = utils::TableResolver::new(&tables, &alias_map, cte_columns);

    for child in node.children() {
        // The star check comes first: on googlesql a `*` is itself an
        // `ASTSelectColumn`, which would otherwise match the column branch.
        if utils::is_star_select_item(&child) {
            let position = child.start_position();
            columns.extend(expand_asterisk(position, &tables, cte_columns));
        } else if child.kind() == "ASTSelectColumn" {
            let column_info = extract_column_info_from_select_expression(&child, sql, &resolver);
            columns.push(column_info);
        }
    }

    columns
}

/// Extract column information from a select_expression node
fn extract_column_info_from_select_expression(
    select_expr: &NodeRef<'_>,
    sql: &str,
    resolver: &utils::TableResolver,
) -> ColumnInfo {
    // Check if there's an alias child (`ASTAlias`)
    let as_alias_node = select_expr
        .children()
        .into_iter()
        .find(|n| n.kind() == "ASTAlias");

    // Extract column information (with or without alias)
    let (column, original_column, source_column) = as_alias_node.map_or_else(
        || extract_column_name_without_alias(select_expr, sql),
        |alias_node| extract_alias_info(&alias_node, select_expr, sql),
    );

    // Resolve the table for this column
    let table = resolver.resolve(&original_column);

    ColumnInfo::new(
        Some(table),
        column,
        source_column,
        select_expr.start_position().row,
        select_expr.start_position().column,
    )
}

/// Extract column name information when there is no alias
fn extract_column_name_without_alias(
    select_expr: &NodeRef<'_>,
    sql: &str,
) -> (String, String, Option<String>) {
    let expr = get_node_text(select_expr, sql).to_string();
    let column_name = utils::extract_column_name(&expr).to_string();
    let source = if column_name != expr {
        Some(column_name.clone())
    } else {
        None
    };
    (column_name, expr, source)
}

/// Extract alias information from an as_alias node
fn extract_alias_info(
    alias_node: &NodeRef<'_>,
    select_expr: &NodeRef<'_>,
    sql: &str,
) -> (String, String, Option<String>) {
    // Extract alias name from as_alias node (last named child)
    let alias_name = alias_node
        .named_children()
        .into_iter()
        .last()
        .map(|n| get_node_text(&n, sql).to_string())
        .unwrap_or_default();

    // Extract original column (first named child of select_expression)
    let original = select_expr
        .named_child(0)
        .map(|n| get_node_text(&n, sql).to_string())
        .unwrap_or_else(|| alias_name.clone());

    // source_column is the base column name without table prefix
    let source = utils::extract_column_name(&original).to_string();
    (alias_name, original, Some(source))
}

/// Expand SELECT * into individual columns
fn expand_asterisk(
    position: Point,
    cte_names: &[String],
    cte_columns: &std::collections::HashMap<String, Vec<ColumnInfo>>,
) -> Vec<ColumnInfo> {
    let mut expanded_columns = Vec::new();
    for cte_name in cte_names {
        if let Some(cols) = cte_columns.get(cte_name) {
            for col in cols {
                let mut cloned_col = col.clone();
                cloned_col.table_name = Some(cte_name.clone());
                cloned_col.row = position.row.saturating_add(1);
                cloned_col.col = position.column.saturating_add(1);
                expanded_columns.push(cloned_col);
            }
        }
    }
    expanded_columns
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

    #[test]
    fn test_cte_visitor() {
        let sql = "WITH cte1 AS (SELECT col1, col2 FROM table1) SELECT * FROM cte1";
        let ast = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let visitor = CteVisitor;

        // Visit all nodes
        for node in ast.pre_order() {
            visitor.visit(node, &mut context);
        }

        assert!(context.has_cte("cte1"));
        let columns = context.get_cte_columns("cte1").unwrap();
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].column_name, "col1");
        assert_eq!(columns[1].column_name, "col2");
    }

    #[test]
    fn test_cte_visitor_with_aliases() {
        let sql = "WITH cte1 AS (SELECT col1 AS alias1, col2 AS alias2, col3 FROM table1) SELECT * FROM cte1";
        let ast = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let visitor = CteVisitor;

        for node in ast.pre_order() {
            visitor.visit(node, &mut context);
        }

        assert!(context.has_cte("cte1"));
        let columns = context.get_cte_columns("cte1").unwrap();
        assert_eq!(columns.len(), 3);
        assert_eq!(columns[0].column_name, "alias1");
        assert_eq!(columns[0].original_column_name, Some("col1".to_string()));
        assert_eq!(columns[1].column_name, "alias2");
        assert_eq!(columns[1].original_column_name, Some("col2".to_string()));
        assert_eq!(columns[2].column_name, "col3");
    }

    #[test]
    fn test_cte_visitor_with_select_star() {
        let sql = "WITH cte1 AS (SELECT col1, col2, col3 FROM table1), \
                   cte2 AS (SELECT * FROM cte1) \
                   SELECT * FROM cte2";
        let ast = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let visitor = CteVisitor;

        for node in ast.pre_order() {
            visitor.visit(node, &mut context);
        }

        // cte1 should have 3 columns
        assert!(context.has_cte("cte1"));
        let columns1 = context.get_cte_columns("cte1").unwrap();
        assert_eq!(columns1.len(), 3);

        // cte2 should expand SELECT * from cte1, so it should also have 3 columns
        assert!(context.has_cte("cte2"));
        let columns2 = context.get_cte_columns("cte2").unwrap();
        assert_eq!(columns2.len(), 3);
        assert_eq!(columns2[0].column_name, "col1");
        assert_eq!(columns2[1].column_name, "col2");
        assert_eq!(columns2[2].column_name, "col3");
    }

    #[test]
    fn test_cte_visitor_multiple_ctes() {
        let sql = "WITH \
                   cte1 AS (SELECT id, name FROM users), \
                   cte2 AS (SELECT user_id, email FROM contacts), \
                   cte3 AS (SELECT order_id, amount FROM orders) \
                   SELECT * FROM cte1";
        let ast = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let visitor = CteVisitor;

        for node in ast.pre_order() {
            visitor.visit(node, &mut context);
        }

        // All three CTEs should be collected
        assert!(context.has_cte("cte1"));
        assert!(context.has_cte("cte2"));
        assert!(context.has_cte("cte3"));

        let columns1 = context.get_cte_columns("cte1").unwrap();
        assert_eq!(columns1.len(), 2);
        assert_eq!(columns1[0].column_name, "id");
        assert_eq!(columns1[1].column_name, "name");

        let columns2 = context.get_cte_columns("cte2").unwrap();
        assert_eq!(columns2.len(), 2);
        assert_eq!(columns2[0].column_name, "user_id");
        assert_eq!(columns2[1].column_name, "email");

        let columns3 = context.get_cte_columns("cte3").unwrap();
        assert_eq!(columns3.len(), 2);
        assert_eq!(columns3[0].column_name, "order_id");
        assert_eq!(columns3[1].column_name, "amount");
    }

    #[test]
    fn test_cte_visitor_with_qualified_names() {
        let sql = "WITH cte1 AS (SELECT t.col1, t.col2 FROM table1 t) SELECT * FROM cte1";
        let ast = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let visitor = CteVisitor;

        for node in ast.pre_order() {
            visitor.visit(node, &mut context);
        }

        assert!(context.has_cte("cte1"));
        let columns = context.get_cte_columns("cte1").unwrap();
        assert_eq!(columns.len(), 2);
        // Qualified names should be simplified to just column names
        assert_eq!(columns[0].column_name, "col1");
        assert_eq!(columns[1].column_name, "col2");
    }

    #[test]
    fn test_cte_visitor_mixed_columns_and_star() {
        let sql = "WITH cte1 AS (SELECT col1, col2 FROM table1), \
                   cte2 AS (SELECT *, col3 FROM cte1, table2) \
                   SELECT * FROM cte2";
        let ast = parse_sql(sql);
        let mut context = AnalysisContext::new(sql);

        let visitor = CteVisitor;

        for node in ast.pre_order() {
            visitor.visit(node, &mut context);
        }

        assert!(context.has_cte("cte1"));
        assert!(context.has_cte("cte2"));

        let columns2 = context.get_cte_columns("cte2").unwrap();
        // Should have columns from cte1 (col1, col2) plus col3
        assert!(columns2.len() >= 3);
    }
}
