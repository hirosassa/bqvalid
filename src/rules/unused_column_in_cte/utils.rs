use std::collections::HashMap;
use tree_sitter::Node;
use tree_sitter_traversal::{Order, traverse};

use crate::rules::helpers::{get_node_text, is_function_name};

use super::context::AnalysisContext;
use super::models::ColumnInfo;

/// Extract CTE name from a CTE node.
///
/// A well-formed CTE always has an `alias_name` field. On a malformed tree
/// where it is missing we return an empty name rather than panicking; an empty
/// CTE name simply fails to match downstream lookups.
pub fn get_cte_name<'a>(cte_node: &Node, sql: &'a str) -> &'a str {
    cte_node
        .child_by_field_name("alias_name")
        .map_or("", |alias_node| get_node_text(&alias_node, sql))
}

/// Extract column name from a potentially qualified column reference
/// e.g., "table.column" -> "column", "column" -> "column"
pub fn extract_column_name(column_ref: &str) -> &str {
    column_ref.split('.').next_back().unwrap_or(column_ref)
}

/// Extract table name from a potentially qualified table reference
/// e.g., "schema.table" -> "schema", "table" -> "table"
pub fn extract_table_name(table_ref: &str) -> &str {
    table_ref.split('.').next().unwrap_or(table_ref)
}

/// Extract tables and aliases from a FROM clause
pub fn extract_table(from: Option<Node>, sql: &str) -> (Vec<String>, HashMap<String, String>) {
    let mut tables = Vec::new();
    let mut alias_map = HashMap::new();

    if let Some(from_node) = from {
        for n in traverse(from_node.walk(), Order::Pre) {
            if n.kind() == "from_item"
                && let Some(first_child) = n.named_child(0)
                && first_child.kind() == "identifier"
            {
                let table_name = get_node_text(&first_child, sql).to_string();
                tables.push(table_name.clone());

                // Check if there's an alias
                for child in n.children(&mut n.walk()) {
                    if child.kind() == "as_alias" {
                        if let Some(alias_node) = child.named_children(&mut child.walk()).last() {
                            let alias_name = get_node_text(&alias_node, sql).to_string();
                            alias_map.insert(alias_name, table_name.clone());
                        }
                        break;
                    }
                }
            }
        }
    }

    (tables, alias_map)
}

/// Find the original table that a column belongs to
pub fn find_original_table(
    column: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> String {
    // If column is qualified (e.g., "table1.column"), extract table name
    if column.contains('.') {
        let table_name = column.split('.').next().unwrap_or("");
        // Check if this is an alias, and resolve it to the actual table name
        let actual_table_name = alias_map
            .get(table_name)
            .map(|s| s.as_str())
            .unwrap_or(table_name);
        if tables.contains(&actual_table_name.to_string())
            || cte_columns.contains_key(actual_table_name)
        {
            return actual_table_name.to_string();
        }
    }

    // For unqualified columns, find by exact column name match
    let column_base_name = extract_column_name(column);
    for table in tables {
        if let Some(columns) = cte_columns.get(table) {
            for column_info in columns {
                let col_base_name = extract_column_name(&column_info.column_name);
                if col_base_name == column_base_name {
                    return table.clone();
                }
            }
        } else {
            return table.clone();
        }
    }
    String::new()
}

/// Recursively extract all field/identifier references and mark them as used
/// This function processes field, identifier, and input_column nodes
pub fn extract_and_mark_fields(
    node: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    context: &mut AnalysisContext,
) {
    // Process current node if it's a field, identifier, or input_column
    if node.kind() == "field" || node.kind() == "identifier" || node.kind() == "input_column" {
        // Skip function names
        if is_function_name(node) {
            return;
        }

        let field_text = get_node_text(node, sql);
        let col_name = extract_column_name(field_text);

        // Find which table this column belongs to
        let table = find_original_table(field_text, tables, alias_map, &context.cte_columns);

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
    fn test_extract_column_name() {
        assert_eq!(extract_column_name("column"), "column");
        assert_eq!(extract_column_name("table.column"), "column");
        assert_eq!(extract_column_name("schema.table.column"), "column");
    }

    #[test]
    fn test_extract_table_name() {
        assert_eq!(extract_table_name("table"), "table");
        assert_eq!(extract_table_name("schema.table"), "schema");
    }

    fn col(name: &str) -> ColumnInfo {
        ColumnInfo::new(None, name.to_string(), None, 0, 0)
    }

    #[test]
    fn find_original_table_resolves_alias_to_real_table() {
        let tables = vec!["orders".to_string()];
        let mut alias_map = HashMap::new();
        alias_map.insert("o".to_string(), "orders".to_string());
        let cte_columns = HashMap::new();

        assert_eq!(
            find_original_table("o.id", &tables, &alias_map, &cte_columns),
            "orders"
        );
    }

    #[test]
    fn find_original_table_uses_qualified_table_directly() {
        let tables = vec!["users".to_string()];
        let alias_map = HashMap::new();
        let cte_columns = HashMap::new();

        assert_eq!(
            find_original_table("users.id", &tables, &alias_map, &cte_columns),
            "users"
        );
    }

    #[test]
    fn find_original_table_matches_unqualified_column_against_cte() {
        let tables = vec!["cte_a".to_string()];
        let alias_map = HashMap::new();
        let mut cte_columns = HashMap::new();
        cte_columns.insert("cte_a".to_string(), vec![col("name"), col("email")]);

        assert_eq!(
            find_original_table("email", &tables, &alias_map, &cte_columns),
            "cte_a"
        );
    }

    #[test]
    fn find_original_table_returns_empty_when_nothing_matches() {
        let tables: Vec<String> = Vec::new();
        let alias_map = HashMap::new();
        let cte_columns = HashMap::new();

        assert_eq!(
            find_original_table("z.x", &tables, &alias_map, &cte_columns),
            ""
        );
    }

    #[test]
    fn get_cte_name_is_empty_for_a_node_without_alias() {
        // A non-CTE node has no `alias_name` field: we must get "" and not panic.
        let sql = "SELECT 1";
        let tree = parse_sql(sql);
        assert_eq!(get_cte_name(&tree.root_node(), sql), "");
    }
}
