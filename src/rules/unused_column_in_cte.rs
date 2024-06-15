use log::debug;
use std::{cmp::Ord, collections::HashMap, fmt::Display};

use tree_sitter::{Node, Point, Tree};
use tree_sitter_traversal::{traverse, Order};

use crate::diagnostic::Diagnostic;

pub fn check(tree: &Tree, sql: &str) -> Option<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    let root = tree.root_node();

    let unused_columns = unused_columns_in_cte(&root, sql);
    for column in unused_columns {
        diagnostics.push(new_unused_column_warning(&column));
    }

    if diagnostics.is_empty() {
        None
    } else {
        Some(diagnostics)
    }
}

fn new_unused_column_warning(column: &ColumnInfo) -> Diagnostic {
    Diagnostic::new(
        column.row,
        column.col,
        format!("Unused column: {}", column.column_name),
    )
}

fn unused_columns_in_cte(node: &Node, sql: &str) -> Vec<ColumnInfo> {
    let cte_columns = collect_cte_columns(node, sql);
    let final_select_columns = collect_final_select_columns(node, sql, &cte_columns);
    find_unused_columns(cte_columns, final_select_columns)
}

fn find_unused_columns(
    cte_columns: HashMap<String, Vec<ColumnInfo>>,
    final_select_columns: Vec<ColumnInfo>,
) -> Vec<ColumnInfo> {
    let mut used_columns = final_select_columns.clone();

    let mut candidates = Vec::new();
    candidates.extend(final_select_columns.iter());

    while let Some(cand) = candidates.pop() {
        if let Some(key) = &cand.table_name {
            if cte_columns.contains_key(key) {
                let columns = cte_columns.get(key).unwrap();
                for col in columns {
                    if col.column_name == cand.column_name {
                        used_columns.push(col.clone());
                        candidates.push(col);
                    }
                }
            }
        }
    }

    let mut all_columns = cte_columns
        .values()
        .flatten()
        .cloned()
        .collect::<Vec<ColumnInfo>>();
    all_columns.extend(final_select_columns);

    let mut unused_columns = all_columns
        .iter()
        .filter(|c| !used_columns.contains(c))
        .cloned()
        .collect::<Vec<ColumnInfo>>();
    unused_columns.sort();

    unused_columns
}

#[derive(Clone, PartialEq, Eq)]
struct ColumnInfo {
    table_name: Option<String>,
    column_name: String,
    row: usize,
    col: usize,
}

impl ColumnInfo {
    const fn new(table_name: Option<String>, column_name: String, row: usize, col: usize) -> Self {
        Self {
            table_name,
            column_name,
            row: row + 1,
            col: col + 1,
        }
    }
}

impl Display for ColumnInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let table_name = self.table_name.clone().unwrap_or_default();
        write!(
            f,
            "{}:{}:{}:{}",
            table_name, self.column_name, self.row, self.col
        )
    }
}

impl PartialOrd for ColumnInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ColumnInfo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.row.cmp(&other.row).then(self.col.cmp(&other.col))
    }
}

fn collect_final_select_columns(
    node: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> Vec<ColumnInfo> {
    let mut columns = Vec::new();
    if let Some(final_select) = find_final_select(node) {
        columns.extend(extract_columns(&final_select, sql, cte_columns));
    }
    columns
}

fn collect_cte_columns(node: &Node, sql: &str) -> HashMap<String, Vec<ColumnInfo>> {
    let mut cte_columns = HashMap::new();

    debug!("Collecting CTE columns");
    for cte_node in find_ctes(node) {
        let cte_name = get_cte_name(&cte_node, sql);
        let columns = extract_columns(&cte_node, sql, &cte_columns);
        debug!("{}", cte_name);
        for column in &columns {
            debug!("{}", column);
        }
        cte_columns.insert(cte_name, columns);
    }

    cte_columns
}

fn find_ctes<'a>(node: &'a Node<'a>) -> Vec<Node<'a>> {
    let mut cte_nodes = Vec::new();
    for n in traverse(node.walk(), Order::Pre) {
        if n.kind() == "cte" {
            cte_nodes.push(n);
        }
    }

    cte_nodes
}

fn get_cte_name(cte_node: &Node, sql: &str) -> String {
    cte_node
        .child_by_field_name("alias_name")
        .unwrap()
        .utf8_text(sql.as_bytes())
        .unwrap()
        .to_string()
}

fn find_final_select<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    for n in traverse(node.walk(), Order::Pre) {
        if n.kind() == "query_expr" {
            for child in n.named_children(&mut n.walk()) {
                if child.kind() == "select" {
                    return Some(child);
                }
            }
        }
    }

    None
}

fn extract_columns(
    node: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> Vec<ColumnInfo> {
    let mut columns = Vec::new();
    if node.kind() == "select_list" {
        let from = node.next_named_sibling();
        let tables = extract_table(from, sql);
        for child in node.children(&mut node.walk()) {
            if child.kind() == "select_expression" {
                let column = child.utf8_text(sql.as_bytes()).unwrap().to_string();
                let table = find_original_table(&column, &tables, cte_columns);
                columns.push(ColumnInfo::new(
                    Some(table.clone()),
                    column,
                    child.start_position().row,
                    child.start_position().column,
                ));
            } else if child.kind() == "select_all" {
                let position = child.start_position();
                columns.extend(expand_asterisk(position, &tables, cte_columns));
            }
        }
    }

    for child in node.named_children(&mut node.walk()) {
        columns.extend(extract_columns(&child, sql, cte_columns));
    }

    columns
}

fn find_original_table(
    column: &str,
    tables: &Vec<String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> String {
    for table in tables {
        if cte_columns.contains_key(table) {
            for column_info in cte_columns.get(table).unwrap() {
                if column.contains(&column_info.column_name) {
                    return table.clone();
                }
            }
        } else {
            return table.clone();
        }
    }
    "".to_string()
}

fn expand_asterisk(
    position: Point,
    cte_names: &Vec<String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> Vec<ColumnInfo> {
    let mut expanded_columns = Vec::new();
    for cte_name in cte_names {
        if cte_columns.contains_key(cte_name) {
            let cols = cte_columns.get(cte_name).unwrap().clone();
            for col in cols {
                let mut cloned_col = col.clone();
                cloned_col.table_name = Some(cte_name.clone());
                cloned_col.row = position.row;
                cloned_col.col = position.column;
                expanded_columns.push(cloned_col);
            }
        }
    }
    expanded_columns
}

fn extract_table(from: Option<Node>, sql: &str) -> Vec<String> {
    let mut tables = Vec::new();
    if let Some(from_node) = from {
        for n in traverse(from_node.walk(), Order::Pre) {
            if n.kind() == "identifier" {
                tables.push(n.utf8_text(sql.as_bytes()).unwrap().to_string());
            }
        }
    }

    tables
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;

    #[test]
    fn test_unused_columns_in_cte_exists() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/unused_column_in_cte_simple.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();
        let root = tree.root_node();

        let columns = unused_columns_in_cte(&root, &sql);
        assert_eq!(columns.len(), 2);

        let actuals = ["unused_column1", "unused_column2"];
        for (expect, actual) in columns.iter().zip(actuals.iter()) {
            assert_eq!(expect.column_name, actual.to_string());
        }
    }

    #[rstest]
    #[case("./sql/valid.sql")]
    #[case("./sql/valid_cte.sql")]
    fn test_unused_columns_in_cte_not_exists(#[case] sql_file: &str) {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string(sql_file).unwrap();
        let tree = parser.parse(&sql, None).unwrap();
        let root = tree.root_node();

        let columns = unused_columns_in_cte(&root, &sql);
        assert!(columns.is_empty());
    }
}
