use log::debug;
use std::{
    cmp::Ord,
    collections::{HashMap, HashSet, VecDeque},
    fmt::Display,
};

use tree_sitter::{Node, Point, Tree};
use tree_sitter_traversal::{Order, traverse};

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

    let mut cte_graph = build_graph_from_existing_data(&cte_columns);
    mark_used_columns(&mut cte_graph, final_select_columns);

    collect_unmarked_columns(&cte_graph)
}


#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone)]
struct CTENode {
    columns: Vec<ColumnInfo>,
    used_column_names: HashSet<String>,
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

    // Extract columns from JOIN ON conditions
    if node.kind() == "join_condition" {
        // Get tables from the FROM clause for context
        let tables = extract_tables_from_parent(node, sql);
        extract_columns_from_condition(node, sql, &tables, cte_columns, &mut columns);
    }

    for child in node.named_children(&mut node.walk()) {
        columns.extend(extract_columns(&child, sql, cte_columns));
    }

    columns
}

fn extract_tables_from_parent(node: &Node, sql: &str) -> Vec<String> {
    // Walk up to find the FROM clause
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "from_clause" {
            return extract_table(Some(parent), sql);
        }
        current = parent.parent();
    }
    Vec::new()
}

fn extract_columns_from_condition(
    node: &Node,
    sql: &str,
    tables: &[String],
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    columns: &mut Vec<ColumnInfo>,
) {
    let actual_tables: Vec<String> = tables
        .iter()
        .filter(|t| !t.contains('.'))
        .cloned()
        .collect();

    // Traverse the condition tree to find all column references
    for child in traverse(node.walk(), Order::Pre) {
        if child.kind() == "field" || child.kind() == "identifier" {
            let column_text = child.utf8_text(sql.as_bytes()).unwrap().to_string();

            // If column reference has a table prefix (e.g., "data1.id"), use that directly
            let table = if column_text.contains('.') {
                column_text.split('.').next().unwrap_or("").to_string()
            } else {
                find_original_table(&column_text, &actual_tables, cte_columns)
            };

            if !table.is_empty() {
                columns.push(ColumnInfo::new(
                    Some(table),
                    column_text,
                    child.start_position().row,
                    child.start_position().column,
                ));
            }
        }
    }
}

fn find_original_table(
    column: &str,
    tables: &Vec<String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> String {
    for table in tables {
        if let Some(columns) = cte_columns.get(table) {
            for column_info in columns {
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
        if let Some(cols) = cte_columns.get(cte_name) {
            for col in cols {
                let mut cloned_col = col.clone();
                cloned_col.table_name = Some(cte_name.clone()); // update table name
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

fn build_graph_from_existing_data(
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> HashMap<String, CTENode> {
    let mut graph = HashMap::new();

    for (cte_name, columns) in cte_columns {
        let cte_node = CTENode {
            columns: columns.clone(),
            used_column_names: HashSet::new(),
        };

        graph.insert(cte_name.clone(), cte_node);
    }

    graph
}

fn mark_used_columns(graph: &mut HashMap<String, CTENode>, final_columns: Vec<ColumnInfo>) {
    let mut queue: VecDeque<(String, String)> = VecDeque::new();

    // Add final select columns to queue
    for col in final_columns {
        if let Some(table_name) = col.table_name {
            queue.push_back((table_name, col.column_name));
        }
    }

    // Process queue until empty
    while let Some((table_name, column_name)) = queue.pop_front() {
        if let Some(cte_node) = graph.get(&table_name).cloned() {
            if should_skip_column(&cte_node, &column_name) {
                continue;
            }

            mark_column_as_used(graph, &table_name, &column_name);
            trace_column_dependencies(&cte_node, &column_name, graph, &mut queue);
        }
    }
}

fn should_skip_column(cte_node: &CTENode, column_name: &str) -> bool {
    cte_node.used_column_names.contains(column_name)
}

fn mark_column_as_used(
    graph: &mut HashMap<String, CTENode>,
    table_name: &str,
    column_name: &str,
) {
    if let Some(node) = graph.get_mut(table_name) {
        node.used_column_names.insert(column_name.to_string());
    }
}

fn trace_column_dependencies(
    cte_node: &CTENode,
    column_name: &str,
    graph: &HashMap<String, CTENode>,
    queue: &mut VecDeque<(String, String)>,
) {
    for col_info in &cte_node.columns {
        if is_column_match(col_info, column_name)
            && let Some(source_table) = &col_info.table_name
        {
            let actual_source_table = extract_table_name(source_table);

            // Only trace back if source_table is also a CTE
            if graph.contains_key(actual_source_table) {
                let search_base_name = extract_column_name(column_name);
                queue.push_back((actual_source_table.to_string(), search_base_name.to_string()));
            }
        }
    }
}

fn is_column_match(col_info: &ColumnInfo, search_column: &str) -> bool {
    let col_base_name = extract_column_name(&col_info.column_name);
    let search_base_name = extract_column_name(search_column);
    col_base_name == search_base_name || col_info.column_name == search_column
}

fn extract_column_name(column_ref: &str) -> &str {
    column_ref.split('.').next_back().unwrap_or(column_ref)
}

fn extract_table_name(table_ref: &str) -> &str {
    table_ref.split('.').next().unwrap_or(table_ref)
}


fn collect_unmarked_columns(graph: &HashMap<String, CTENode>) -> Vec<ColumnInfo> {
    let mut unused = Vec::new();

    for cte_node in graph.values() {
        for col in &cte_node.columns {
            // Check if this column is marked as used
            if !cte_node.used_column_names.contains(&col.column_name) {
                unused.push(col.clone());
            }
        }
    }

    unused.sort();
    unused
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;

    #[rstest]
    #[case("./sql/unused_column_in_cte_simple.sql", 2, vec!["unused_column1", "unused_column2"])]
    #[case("./sql/unused_column_in_cte_complex.sql", 6, vec!["unused_field1", "unused_field2", "unused_amount_field", "price", "unused_price_field", "another_unused"])]
    #[case("./sql/unused_column_in_cte_join_only.sql", 1, vec!["unused_field"])]
    fn test_unused_columns_in_cte_exists(
        #[case] sql_file: &str,
        #[case] expected_count: usize,
        #[case] expected_names: Vec<&str>,
    ) {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string(sql_file).unwrap();
        let tree = parser.parse(&sql, None).unwrap();
        let root = tree.root_node();

        let columns = unused_columns_in_cte(&root, &sql);
        assert_eq!(columns.len(), expected_count);

        for (expected, actual) in expected_names.iter().zip(columns.iter()) {
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
        // Now includes JOIN condition columns (data1.column1 from ON clause)
        assert_eq!(final_select_columns.len(), 5);

        // Check the first few columns (from SELECT *)
        let expected_columns_from_select = vec!["column1", "column2", "column3"];
        for (i, expected) in expected_columns_from_select.iter().enumerate() {
            assert_eq!("data3", final_select_columns[i].table_name.clone().unwrap());
            assert_eq!(*expected.to_string(), final_select_columns[i].column_name);
        }
    }

}
