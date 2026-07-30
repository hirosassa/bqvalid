use std::collections::HashMap;

use crate::ast::NodeRef;
use crate::rules::helpers::{get_node_text, is_function_name};

use super::context::AnalysisContext;
use super::models::ColumnInfo;

/// Extract CTE name from a CTE node.
///
/// A well-formed CTE always has an `alias_name` field. On a malformed tree
/// where it is missing we return an empty name rather than panicking; an empty
/// CTE name simply fails to match downstream lookups.
pub fn get_cte_name<'a>(cte_node: &NodeRef<'_>, sql: &'a str) -> &'a str {
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
pub fn extract_table(
    from: Option<NodeRef<'_>>,
    sql: &str,
) -> (Vec<String>, HashMap<String, String>) {
    let mut tables = Vec::new();
    let mut alias_map = HashMap::new();

    if let Some(from_node) = from {
        for n in from_node.pre_order() {
            if n.kind() == "from_item"
                && let Some(first_child) = n.named_child(0)
                && first_child.kind() == "identifier"
            {
                let table_name = get_node_text(&first_child, sql).to_string();
                tables.push(table_name.clone());

                // Check if there's an alias
                for child in n.children() {
                    if child.kind() == "as_alias" {
                        if let Some(alias_node) = child.named_children().into_iter().last() {
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

/// Resolves column references to their owning table within a single query scope
/// (one FROM clause).
///
/// The unqualified-column lookup that [`find_original_table`] performs is
/// `O(tables × columns_per_table)` per call, and each scope resolves many
/// columns. `TableResolver` precomputes that mapping once so subsequent lookups
/// are `O(1)`, while reproducing the exact resolution order of the original
/// scan:
/// - tables are considered in FROM order;
/// - a table that is not a known CTE has unknown columns and therefore acts as
///   a wildcard that claims any column, short-circuiting every table after it;
/// - among CTEs preceding that wildcard, the first to expose a column wins.
pub struct TableResolver<'a> {
    tables: &'a [String],
    alias_map: &'a HashMap<String, String>,
    // Derived from `cte_columns`, but owned so the resolver only borrows the
    // (context-independent) FROM-clause data and can coexist with a mutable
    // borrow of the analysis context at the call sites.
    is_cte: std::collections::HashSet<String>,
    /// base column name -> owning table, for CTEs before the first wildcard.
    unqualified: HashMap<String, &'a str>,
    /// first non-CTE table in FROM order, if any; claims otherwise-unmatched columns.
    wildcard: Option<&'a str>,
}

impl<'a> TableResolver<'a> {
    pub fn new(
        tables: &'a [String],
        alias_map: &'a HashMap<String, String>,
        cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    ) -> Self {
        let mut is_cte = std::collections::HashSet::new();
        let mut unqualified: HashMap<String, &str> = HashMap::new();
        let mut wildcard = None;

        for key in cte_columns.keys() {
            is_cte.insert(key.clone());
        }

        for table in tables {
            match cte_columns.get(table) {
                Some(columns) => {
                    for column_info in columns {
                        let base = extract_column_name(&column_info.column_name);
                        // First table in FROM order wins on a collision.
                        unqualified
                            .entry(base.to_string())
                            .or_insert(table.as_str());
                    }
                }
                None => {
                    // Unknown columns: this table claims everything not already
                    // matched, and shadows all following tables.
                    wildcard = Some(table.as_str());
                    break;
                }
            }
        }

        Self {
            tables,
            alias_map,
            is_cte,
            unqualified,
            wildcard,
        }
    }

    /// Find the original table that a column belongs to, or `""` if none.
    pub fn resolve(&self, column: &str) -> String {
        // If column is qualified (e.g., "table1.column"), try the table prefix.
        if column.contains('.') {
            let table_name = column.split('.').next().unwrap_or("");
            // Resolve aliases to the actual table name.
            let actual_table_name = self
                .alias_map
                .get(table_name)
                .map(|s| s.as_str())
                .unwrap_or(table_name);
            if self.tables.iter().any(|t| t == actual_table_name)
                || self.is_cte.contains(actual_table_name)
            {
                return actual_table_name.to_string();
            }
        }

        // For unqualified columns (or qualified ones with an unknown prefix),
        // match by the trailing column name.
        let base = extract_column_name(column);
        if let Some(table) = self.unqualified.get(base) {
            return (*table).to_string();
        }
        if let Some(wildcard) = self.wildcard {
            return wildcard.to_string();
        }
        String::new()
    }

    /// Resolve using the more permissive rule that WHERE / JOIN / UNNEST column
    /// references use: a qualified prefix is always taken (alias-resolved),
    /// even when it names neither a known table nor a CTE. Unqualified columns
    /// fall back to [`resolve`](Self::resolve).
    pub fn resolve_qualified_or(&self, column: &str) -> String {
        if column.contains('.') {
            let prefix = column.split('.').next().unwrap_or("");
            return self
                .alias_map
                .get(prefix)
                .cloned()
                .unwrap_or_else(|| prefix.to_string());
        }
        self.resolve(column)
    }
}

/// Recursively extract all field/identifier references and mark them as used
/// This function processes field, identifier, and input_column nodes
pub fn extract_and_mark_fields(
    node: &NodeRef<'_>,
    sql: &str,
    resolver: &TableResolver,
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
        let table = resolver.resolve(field_text);

        if !table.is_empty() {
            context.mark_used(&table, col_name);
        }
    }

    // Recursively process children
    for child in node.children() {
        extract_and_mark_fields(&child, sql, resolver, context);
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

    /// Test helper: resolve a single column via a one-shot [`TableResolver`].
    fn find_original_table(
        column: &str,
        tables: &[String],
        alias_map: &HashMap<String, String>,
        cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    ) -> String {
        TableResolver::new(tables, alias_map, cte_columns).resolve(column)
    }

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

    // The following tests lock the order-dependent quirks of resolution so the
    // memoized implementation stays behaviourally identical.

    #[test]
    fn find_original_table_treats_non_cte_table_as_wildcard() {
        // A table that is not a known CTE has unknown columns, so *any*
        // unqualified column is attributed to it.
        let tables = vec!["physical".to_string()];
        let alias_map = HashMap::new();
        let cte_columns = HashMap::new();

        assert_eq!(
            find_original_table("anything", &tables, &alias_map, &cte_columns),
            "physical"
        );
    }

    #[test]
    fn find_original_table_prefers_first_cte_on_column_collision() {
        // When two CTEs both expose the column, the first table in FROM wins.
        let tables = vec!["a".to_string(), "b".to_string()];
        let alias_map = HashMap::new();
        let mut cte_columns = HashMap::new();
        cte_columns.insert("a".to_string(), vec![col("x")]);
        cte_columns.insert("b".to_string(), vec![col("x")]);

        assert_eq!(
            find_original_table("x", &tables, &alias_map, &cte_columns),
            "a"
        );
    }

    #[test]
    fn find_original_table_falls_through_to_later_cte_without_match() {
        let tables = vec!["a".to_string(), "b".to_string()];
        let alias_map = HashMap::new();
        let mut cte_columns = HashMap::new();
        cte_columns.insert("a".to_string(), vec![col("p")]);
        cte_columns.insert("b".to_string(), vec![col("x")]);

        assert_eq!(
            find_original_table("x", &tables, &alias_map, &cte_columns),
            "b"
        );
    }

    #[test]
    fn find_original_table_wildcard_shadows_following_tables() {
        // A non-CTE table appearing before a CTE short-circuits resolution:
        // the non-CTE table is returned even for a column the later CTE owns.
        let tables = vec!["physical".to_string(), "cte_b".to_string()];
        let alias_map = HashMap::new();
        let mut cte_columns = HashMap::new();
        cte_columns.insert("cte_b".to_string(), vec![col("x")]);

        assert_eq!(
            find_original_table("x", &tables, &alias_map, &cte_columns),
            "physical"
        );
    }

    #[test]
    fn find_original_table_cte_takes_precedence_over_following_wildcard() {
        let tables = vec!["cte_a".to_string(), "physical".to_string()];
        let alias_map = HashMap::new();
        let mut cte_columns = HashMap::new();
        cte_columns.insert("cte_a".to_string(), vec![col("x")]);

        // Column owned by the CTE resolves to the CTE...
        assert_eq!(
            find_original_table("x", &tables, &alias_map, &cte_columns),
            "cte_a"
        );
        // ...but an unknown column falls through to the wildcard table.
        assert_eq!(
            find_original_table("y", &tables, &alias_map, &cte_columns),
            "physical"
        );
    }

    #[test]
    fn find_original_table_falls_through_when_qualifier_is_unknown() {
        // Qualified reference whose prefix is neither a table nor an alias:
        // resolution falls back to matching the trailing column name.
        let tables = vec!["cte_a".to_string()];
        let alias_map = HashMap::new();
        let mut cte_columns = HashMap::new();
        cte_columns.insert("cte_a".to_string(), vec![col("x")]);

        assert_eq!(
            find_original_table("unknown.x", &tables, &alias_map, &cte_columns),
            "cte_a"
        );
    }

    #[test]
    fn get_cte_name_is_empty_for_a_node_without_alias() {
        // A non-CTE node has no `alias_name` field: we must get "" and not panic.
        let sql = "SELECT 1";
        let ast = parse_sql(sql);
        assert_eq!(get_cte_name(&ast.root(), sql), "");
    }
}
