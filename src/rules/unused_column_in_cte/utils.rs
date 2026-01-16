use std::collections::HashMap;
use tree_sitter::Node;
use tree_sitter_traversal::{Order, traverse};

use super::models::ColumnInfo;

/// Extract CTE name from a CTE node
pub fn get_cte_name<'a>(cte_node: &Node, sql: &'a str) -> &'a str {
    cte_node
        .child_by_field_name("alias_name")
        .unwrap()
        .utf8_text(sql.as_bytes())
        .unwrap()
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
                let table_name = first_child.utf8_text(sql.as_bytes()).unwrap().to_string();
                tables.push(table_name.clone());

                // Check if there's an alias
                for child in n.children(&mut n.walk()) {
                    if child.kind() == "as_alias" {
                        if let Some(alias_node) = child.named_children(&mut child.walk()).last() {
                            let alias_name =
                                alias_node.utf8_text(sql.as_bytes()).unwrap().to_string();
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
