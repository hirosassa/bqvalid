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
    let mut candidates = final_select_columns.clone();

    while let Some(cand) = candidates.pop() {
        if let Some(key) = &cand.table_name {
            if let Some(columns) = cte_columns.get(key) {
                for col in columns {
                    if col.column_name == cand.column_name {
                        used_columns.push(col.clone());
                        candidates.push(col.clone());
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
        debug!("Collecting Final select columns");
        for column in columns.iter() {
            debug!("{}", column);
        }
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
            debug!("\t{}", column);
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
        let from_clause = node.next_named_sibling();
        let tables = extract_tables(from_clause, sql);

        for child in node.children(&mut node.walk()) {
            if child.kind() == "select_expression" {
                let column_identifier = child.utf8_text(sql.as_bytes()).unwrap();
                let (table_alias, column) = parse_column(column_identifier);
                let table = find_original_table(column, table_alias, &tables, cte_columns);
                columns.push(ColumnInfo::new(
                    Some(table.clone()),
                    column.to_string(),
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

fn parse_column(column: &str) -> (&str, &str) {
    if column.contains('.') {
        // "alias.column_name" case
        let mut iter = column.split('.');
        let table_alias = iter.next().unwrap_or_default();
        let column_name = iter.next().unwrap_or_default();
        return (table_alias, column_name);
    }
    ("", column)
}

fn find_original_table(
    column: &str,
    table_alias: &str,
    tables: &Vec<TableInfo>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> String {
    for table in tables {
        // table alias is specified
        if let Some(alias) = &table.alias_name {
            if alias == table_alias {
                return table.table_name.clone();
            }
        }

        // The column comes from the CTE
        if let Some(columns) = cte_columns.get(table.table_name.as_str()) {
            for column_info in columns {
                if column == column_info.column_name {
                    return table.table_name.clone();
                }
            }
        }
    }

    // The column is constant value e.g. "select 1"
    "".to_string()
}

fn expand_asterisk(
    position: Point,
    tables: &Vec<TableInfo>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> Vec<ColumnInfo> {
    let mut expanded_columns = Vec::new();
    for table in tables {
        if let Some(cols) = cte_columns.get(table.table_name.as_str()) {
            for col in cols {
                let mut cloned_col = col.clone();
                cloned_col.table_name = Some(table.table_name.to_string());
                cloned_col.row = position.row;
                cloned_col.col = position.column;
                expanded_columns.push(cloned_col);
            }
        }
    }
    expanded_columns
}

struct TableInfo {
    table_name: String,
    alias_name: Option<String>,
}

impl TableInfo {
    const fn new(table_name: String, alias_name: Option<String>) -> Self {
        Self {
            table_name,
            alias_name,
        }
    }
}

fn extract_tables(from_clause: Option<Node>, sql: &str) -> Vec<TableInfo> {
    if from_clause.is_none() {
        return Vec::new();
    }

    let mut tables = Vec::new();
    let from_node = from_clause.unwrap();

    // list "(from_item table_name: (identifier) alias_name: (identifier)))" structure
    // under the "from_clause" subtree
    for node in traverse(from_node.walk(), Order::Post) {
        if node.kind() == "from_item" {
            if let Some(table_node) = node.child_by_field_name("table_name") {
                tables.push(parse_table(table_node, sql));
            }
        }
    }

    tables
}

fn parse_table(table_node: Node, sql: &str) -> TableInfo {
    let table_name = table_node.utf8_text(sql.as_bytes()).unwrap().to_string();
    let alias_name = table_node
        .child_by_field_name("alias_name")
        .map(|alias_node| alias_node.utf8_text(sql.as_bytes()).unwrap().to_string());

    TableInfo::new(table_name, alias_name)
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

        let expecteds = ["unused_column1", "unused_column2"];
        for (expected, actual) in expecteds.iter().zip(columns.iter()) {
            assert_eq!(*expected.to_string(), actual.column_name);
        }
    }

    #[test]
    fn test_unused_columns_in_cte_with_table_alias() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/unused_column_in_cte_with_table_alias.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();
        let root = tree.root_node();

        let columns = unused_columns_in_cte(&root, &sql);
        assert_eq!(columns.len(), 2);

        let expecteds = ["unused_column_1", "unused_column_1"];
        for (expected, actual) in expecteds.iter().zip(columns.iter()) {
            assert_eq!(*expected.to_string(), actual.column_name);
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

    #[test]
    fn test_collect_cte_columns() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/unused_column_in_cte_simple.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();
        let root = tree.root_node();

        let cte_columns = collect_cte_columns(&root, &sql);
        assert_eq!(cte_columns.len(), 3);

        let cte_names = ["data1", "data2", "data3"];
        let cte_column_names = [
            vec!["column1", "column2", "unused_column1"],
            vec!["column3", "unused_column2"],
            vec!["column1", "column2", "column3"],
        ];
        for (cte_name, column_names) in cte_names.iter().zip(cte_column_names.iter()) {
            assert!(cte_columns.contains_key(*cte_name));

            let actual_columns = cte_columns.get(*cte_name).unwrap();
            for (expect, actual) in column_names.iter().zip(actual_columns.iter()) {
                assert_eq!(*expect.to_string(), actual.column_name);
            }
        }
    }

    #[test]
    fn test_collect_final_select_columns() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/unused_column_in_cte_simple.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();
        let root = tree.root_node();

        let cte_columns = collect_cte_columns(&root, &sql);
        let final_select_columns = collect_final_select_columns(&root, &sql, &cte_columns);
        assert_eq!(final_select_columns.len(), 3);

        let expected_table = "data3";
        let expected_columns = ["column1", "column2", "column3"];
        for (expected, actual) in expected_columns.iter().zip(final_select_columns.iter()) {
            assert_eq!(expected_table, actual.table_name.clone().unwrap());
            assert_eq!(*expected.to_string(), actual.column_name);
        }
    }

    #[test]
    fn test_find_unused_columns() {
        let cte_columns = HashMap::from([
            (
                "data1".to_string(),
                vec![
                    ColumnInfo::new(Some("table1".to_string()), "column1".to_string(), 3, 5),
                    ColumnInfo::new(Some("table1".to_string()), "column2".to_string(), 4, 5),
                    ColumnInfo::new(
                        Some("table1".to_string()),
                        "unused_column1".to_string(),
                        5,
                        5,
                    ),
                ],
            ),
            (
                "data2".to_string(),
                vec![
                    ColumnInfo::new(Some("table2".to_string()), "column3".to_string(), 10, 5),
                    ColumnInfo::new(
                        Some("table2".to_string()),
                        "unused_column2".to_string(),
                        11,
                        5,
                    ),
                ],
            ),
            (
                "data3".to_string(),
                vec![
                    ColumnInfo::new(Some("data1".to_string()), "column1".to_string(), 16, 5),
                    ColumnInfo::new(Some("data1".to_string()), "column2".to_string(), 17, 5),
                    ColumnInfo::new(Some("data2".to_string()), "column3".to_string(), 18, 5),
                ],
            ),
        ]);
        let final_select_columns = vec![
            ColumnInfo::new(Some("data3".to_string()), "column1".to_string(), 6, 1),
            ColumnInfo::new(Some("data3".to_string()), "column2".to_string(), 7, 1),
            ColumnInfo::new(Some("data3".to_string()), "column3".to_string(), 8, 1),
        ];

        let unused_columns = find_unused_columns(cte_columns, final_select_columns);

        assert_eq!(unused_columns.len(), 2);
        let expecteds = ["unused_column1", "unused_column2"];
        for (expected, actual) in expecteds.iter().zip(unused_columns.iter()) {
            assert_eq!(*expected.to_string(), actual.column_name);
        }
    }
}
