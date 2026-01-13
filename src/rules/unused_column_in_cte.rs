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
    mark_columns_used_in_select_expressions(node, sql, &mut cte_graph);
    mark_columns_used_in_qualify_clauses(node, sql, &mut cte_graph);
    mark_columns_used_by_select_star(node, sql, &mut cte_graph);
    mark_columns_used_in_distinct(node, sql, &mut cte_graph);
    mark_columns_used_in_join_and_where(node, sql, &mut cte_graph);
    mark_columns_used_in_pivot(node, sql, &mut cte_graph);
    mark_columns_used_in_from_clause_subqueries(node, sql, &mut cte_graph);
    mark_used_columns(&mut cte_graph, final_select_columns);

    collect_unmarked_columns(&cte_graph)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ColumnInfo {
    table_name: Option<String>,
    column_name: String,
    /// Original column name before alias (if aliased)
    /// e.g., "column1" for "column1 AS unique_id"
    original_column_name: Option<String>,
    row: usize,
    col: usize,
}

impl ColumnInfo {
    const fn new(
        table_name: Option<String>,
        column_name: String,
        original_column_name: Option<String>,
        row: usize,
        col: usize,
    ) -> Self {
        Self {
            table_name,
            column_name,
            original_column_name,
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
        // Find the select_list within the final SELECT
        let select_list = final_select
            .named_children(&mut final_select.walk())
            .find(|child| child.kind() == "select_list");

        if let Some(list) = select_list {
            columns.extend(extract_columns(&list, sql, cte_columns));
        }
    }
    columns
}

fn collect_cte_columns(node: &Node, sql: &str) -> HashMap<String, Vec<ColumnInfo>> {
    let mut cte_columns = HashMap::new();

    debug!("Collecting CTE columns");
    for cte_node in find_ctes(node) {
        let cte_name = get_cte_name(&cte_node, sql);

        // Find the SELECT or query_expr that defines this CTE
        // CTE structure: cte -> identifier (name) -> select/query_expr
        let mut query_node = None;
        for child in cte_node.named_children(&mut cte_node.walk()) {
            if child.kind() == "select" || child.kind() == "query_expr" {
                query_node = Some(child);
                break;
            }
        }

        let columns = query_node.map_or_else(Vec::new, |query| {
            // For query_expr, find the select child
            let select_node = if query.kind() == "query_expr" {
                query
                    .named_children(&mut query.walk())
                    .find(|c| c.kind() == "select")
            } else {
                Some(query)
            };

            select_node.map_or_else(Vec::new, |sel| {
                // Find the select_list within this SELECT
                let select_list = sel
                    .named_children(&mut sel.walk())
                    .find(|child| child.kind() == "select_list");

                select_list.map_or_else(Vec::new, |list| extract_columns(&list, sql, &cte_columns))
            })
        });

        debug!("{}", cte_name);
        for column in &columns {
            debug!("{}", column);
        }

        cte_columns.insert(cte_name, columns);
    }

    cte_columns
}

/// Mark columns used by SELECT *
/// When a CTE uses SELECT *, all columns from the source tables should be marked as used
fn mark_columns_used_by_select_star(node: &Node, sql: &str, graph: &mut HashMap<String, CTENode>) {
    let cte_columns: HashMap<String, Vec<ColumnInfo>> = graph
        .iter()
        .map(|(name, node)| (name.clone(), node.columns.clone()))
        .collect();

    for cte_node in find_ctes(node) {
        // Find the select_list node in this CTE
        for n in traverse(cte_node.walk(), Order::Pre) {
            if n.kind() == "select_list" {
                // Check if this select_list contains a select_all
                let has_select_star = n
                    .children(&mut n.walk())
                    .any(|child| child.kind() == "select_all");

                if has_select_star {
                    // Get the FROM clause to find source tables
                    let from = n.next_named_sibling();
                    let (tables, _alias_map) = extract_table(from, sql);

                    // Mark all columns from these tables as used
                    for table in &tables {
                        if let Some(cols) = cte_columns.get(table) {
                            for col in cols {
                                mark_column_as_used(graph, table, &col.column_name);
                            }
                        }
                    }
                }
                // Note: We don't break here to process all select_lists in this CTE
                // This is important for UNION queries where multiple SELECT clauses exist
            }
        }
    }
}

/// Mark all columns in SELECT DISTINCT clauses
/// When a CTE uses SELECT DISTINCT, all columns affect the deduplication logic
/// even if they are not explicitly referenced in later queries
fn mark_columns_used_in_distinct(node: &Node, sql: &str, graph: &mut HashMap<String, CTENode>) {
    for cte_node in find_ctes(node) {
        let cte_name = get_cte_name(&cte_node, sql);

        // Find the SELECT node within this CTE
        for n in traverse(cte_node.walk(), Order::Pre) {
            if n.kind() == "select" {
                // Check if this SELECT has a DISTINCT modifier
                // DISTINCT appears as a keyword between SELECT and the select_list
                let has_distinct = n.children(&mut n.walk()).any(|child| {
                    let text = child.utf8_text(sql.as_bytes()).unwrap_or("");
                    text.eq_ignore_ascii_case("distinct")
                });

                if has_distinct {
                    // Mark all columns in this CTE as used
                    if let Some(cte_node) = graph.get(&cte_name) {
                        let columns: Vec<String> = cte_node
                            .columns
                            .iter()
                            .map(|col| col.column_name.clone())
                            .collect();

                        for col_name in columns {
                            mark_column_as_used(graph, &cte_name, &col_name);
                        }
                    }
                }
                // Note: We don't break here to process all SELECT nodes in this CTE
                // This is important for UNION queries where multiple SELECT clauses exist
            }
        }
    }
}

/// Mark a column reference in CTE context
/// Handles both qualified (table.column) and unqualified (column) references
fn mark_column_reference_in_cte_context(
    col_ref: &ColumnInfo,
    cte_name: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    let col_name = extract_column_name(&col_ref.column_name);

    if col_ref.column_name.contains('.') {
        // Qualified column reference (e.g., table.column)
        if let Some(table_name) = &col_ref.table_name {
            mark_column_as_used(graph, table_name, col_name);
        }
    } else {
        // Unqualified column reference - check if it exists in the CTE's SELECT list
        if let Some(cte_cols) = cte_columns.get(cte_name) {
            let exists_in_cte = cte_cols
                .iter()
                .any(|c| extract_column_name(&c.column_name) == col_name);

            if exists_in_cte {
                mark_column_as_used(graph, cte_name, col_name);
            } else if let Some(table_name) = &col_ref.table_name {
                mark_column_as_used(graph, table_name, col_name);
            }
        } else if let Some(table_name) = &col_ref.table_name {
            mark_column_as_used(graph, table_name, col_name);
        }
    }
}

/// Mark a column reference (for final SELECT)
fn mark_column_reference(col_ref: &ColumnInfo, graph: &mut HashMap<String, CTENode>) {
    if let Some(table_name) = &col_ref.table_name {
        let col_name = extract_column_name(&col_ref.column_name);
        mark_column_as_used(graph, table_name, col_name);
    }
}

/// Check if a node is a condition node (JOIN/WHERE/GROUP BY)
fn is_condition_node(node: &Node) -> bool {
    matches!(
        node.kind(),
        "join_condition" | "where_clause" | "group_by_clause"
    )
}

/// Process a condition node in CTE context
fn process_condition_node_in_cte_context(
    node: &Node,
    sql: &str,
    cte_name: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    let (tables, alias_map) = extract_tables_from_parent(node, sql);
    let mut col_refs = Vec::new();
    extract_columns_from_condition(node, sql, &tables, &alias_map, cte_columns, &mut col_refs);

    for col_ref in col_refs {
        mark_column_reference_in_cte_context(&col_ref, cte_name, cte_columns, graph);
    }
}

/// Process a condition node (for final SELECT)
fn process_condition_node(
    node: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    let (tables, alias_map) = extract_tables_from_parent(node, sql);
    let mut col_refs = Vec::new();
    extract_columns_from_condition(node, sql, &tables, &alias_map, cte_columns, &mut col_refs);

    for col_ref in col_refs {
        mark_column_reference(&col_ref, graph);
    }
}

/// Process all condition nodes in a CTE
fn process_condition_nodes_in_cte(
    cte_node: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    let cte_name = get_cte_name(cte_node, sql);

    for n in traverse(cte_node.walk(), Order::Pre) {
        if is_condition_node(&n) {
            process_condition_node_in_cte_context(&n, sql, &cte_name, cte_columns, graph);
        }
    }
}

/// Process all condition nodes in final SELECT
fn process_condition_nodes_in_final_select(
    final_select: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    for n in traverse(final_select.walk(), Order::Pre) {
        if is_condition_node(&n) {
            process_condition_node(&n, sql, cte_columns, graph);
        }
    }
}

/// Find final SELECT node after cte_clause
fn find_final_select_after_cte<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    for n in traverse(node.walk(), Order::Pre) {
        if n.kind() == "query_expr" {
            for child in n.named_children(&mut n.walk()) {
                if child.kind() == "cte_clause" {
                    return child.next_named_sibling();
                }
            }
            break;
        }
    }
    None
}

/// Mark columns used in JOIN conditions, WHERE clauses, and GROUP BY clauses
/// These columns are used for filtering/joining/grouping but not selected in output
fn mark_columns_used_in_join_and_where(
    node: &Node,
    sql: &str,
    graph: &mut HashMap<String, CTENode>,
) {
    let cte_columns: HashMap<String, Vec<ColumnInfo>> = graph
        .iter()
        .map(|(name, node)| (name.clone(), node.columns.clone()))
        .collect();

    for cte_node in find_ctes(node) {
        process_condition_nodes_in_cte(&cte_node, sql, &cte_columns, graph);
    }

    if let Some(final_select) = find_final_select_after_cte(node) {
        process_condition_nodes_in_final_select(&final_select, sql, &cte_columns, graph);
    }
}

/// Check if a node should be processed as a column reference in PIVOT
fn should_process_identifier_in_pivot(node: &Node) -> bool {
    if node.kind() != "identifier" {
        return false;
    }

    let is_function_name = node.parent().is_some_and(|parent| {
        parent.kind() == "function_call"
            && parent
                .child(0)
                .is_some_and(|first_child| first_child.id() == node.id())
    });

    !is_function_name
}

/// Extract column references from a pivot_operator node
fn extract_column_references_from_pivot(
    pivot_node: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> Vec<ColumnInfo> {
    let mut col_refs = Vec::new();

    for child in traverse(pivot_node.walk(), Order::Pre) {
        if should_process_identifier_in_pivot(&child) || child.kind() == "input_column" {
            let column_text = child.utf8_text(sql.as_bytes()).unwrap().to_string();
            let table = find_original_table(&column_text, &tables.to_vec(), alias_map, cte_columns);

            if !table.is_empty() {
                col_refs.push(ColumnInfo::new(
                    Some(table),
                    column_text,
                    None,
                    child.start_position().row,
                    child.start_position().column,
                ));
            }
        }
    }

    col_refs
}

/// Process a pivot_operator node
fn process_pivot_operator_node(
    pivot_node: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    let (tables, alias_map) = extract_tables_from_parent(pivot_node, sql);
    let col_refs =
        extract_column_references_from_pivot(pivot_node, sql, &tables, &alias_map, cte_columns);

    for col_ref in col_refs {
        if let Some(table_name) = &col_ref.table_name {
            let col_name = extract_column_name(&col_ref.column_name);
            mark_column_as_used(graph, table_name, col_name);
        }
    }
}

/// Mark columns that are referenced in PIVOT clauses
/// This ensures that columns used in PIVOT (aggregate expressions and FOR clause) are marked as used
fn mark_columns_used_in_pivot(node: &Node, sql: &str, graph: &mut HashMap<String, CTENode>) {
    let cte_columns: HashMap<String, Vec<ColumnInfo>> = graph
        .iter()
        .map(|(name, node)| (name.clone(), node.columns.clone()))
        .collect();

    for cte_node in find_ctes(node) {
        for n in traverse(cte_node.walk(), Order::Pre) {
            if n.kind() == "pivot_operator" {
                process_pivot_operator_node(&n, sql, &cte_columns, graph);
            }
        }
    }
}

/// Mark columns that are referenced in QUALIFY clauses
/// This ensures that columns used in QUALIFY conditions are marked as used
fn mark_columns_used_in_qualify_clauses(
    node: &Node,
    sql: &str,
    graph: &mut HashMap<String, CTENode>,
) {
    let cte_columns: HashMap<String, Vec<ColumnInfo>> = graph
        .iter()
        .map(|(name, node)| (name.clone(), node.columns.clone()))
        .collect();

    for cte_node in find_ctes(node) {
        // Find the qualify_clause node in this CTE
        for n in traverse(cte_node.walk(), Order::Pre) {
            if n.kind() == "qualify_clause" {
                // Get tables from the SELECT that this QUALIFY belongs to
                let (tables, alias_map) = extract_tables_from_qualify_parent(&n, sql);

                // Extract column references from the QUALIFY clause
                let mut col_refs = Vec::new();
                extract_columns_from_condition(
                    &n,
                    sql,
                    &tables,
                    &alias_map,
                    &cte_columns,
                    &mut col_refs,
                );

                // Mark these column references as used
                for col_ref in col_refs {
                    if let Some(table_name) = &col_ref.table_name {
                        let col_name = extract_column_name(&col_ref.column_name);
                        mark_column_as_used(graph, table_name, col_name);
                    }
                }
                break; // Only process the first qualify_clause in this CTE
            }
        }
    }
}

/// Check if an identifier should be processed in FROM clause context
/// Returns false for function names and UNNEST keyword
fn should_process_identifier_in_from_clause(node: &Node, sql: &str) -> bool {
    if node.kind() != "identifier" {
        return true;
    }

    // Skip function names
    let is_function_name = node.parent().is_some_and(|parent| {
        parent.kind() == "function_call"
            && parent
                .child(0)
                .is_some_and(|first_child| first_child.id() == node.id())
    });

    if is_function_name {
        return false;
    }

    // Skip UNNEST keyword
    let text = node.utf8_text(sql.as_bytes()).unwrap_or("");
    !text.eq_ignore_ascii_case("unnest")
}

/// Extract column references from identifiers and fields in a node
fn extract_and_mark_columns_from_node(
    node: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
    check_identifier: bool,
) {
    for child in traverse(node.walk(), Order::Pre) {
        if child.kind() == "field" || child.kind() == "identifier" {
            if check_identifier && !should_process_identifier_in_from_clause(&child, sql) {
                continue;
            }

            let column_text = child.utf8_text(sql.as_bytes()).unwrap().to_string();
            let table = resolve_table_for_column(&column_text, tables, alias_map, cte_columns);

            if !table.is_empty() {
                let col_name = extract_column_name(&column_text);
                mark_column_as_used(graph, &table, col_name);
            }
        }
    }
}

/// Process subqueries in FROM clause
fn process_subqueries_in_from_clause(
    from_node: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    for child in traverse(from_node.walk(), Order::Pre) {
        if child.kind() == "select" {
            // Extract tables from this subquery's FROM clause
            let mut subquery_tables = Vec::new();
            let mut subquery_alias_map = HashMap::new();

            for subchild in child.named_children(&mut child.walk()) {
                if subchild.kind() == "from_clause" {
                    (subquery_tables, subquery_alias_map) = extract_table(Some(subchild), sql);
                    break;
                }
            }

            // Extract column references from the SELECT list
            for subchild in child.named_children(&mut child.walk()) {
                if subchild.kind() == "select_list" {
                    extract_and_mark_columns_from_node(
                        &subchild,
                        sql,
                        &subquery_tables,
                        &subquery_alias_map,
                        cte_columns,
                        graph,
                        true,
                    );
                    break;
                }
            }
        }
    }
}

/// Process function calls and unnest clauses in FROM clause
fn process_functions_and_unnest_in_from_clause(
    from_node: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    for child in traverse(from_node.walk(), Order::Pre) {
        if child.kind() == "function_call" || child.kind() == "unnest_clause" {
            // For unnest_clause, only process identifiers inside unnest_operator
            let search_root = if child.kind() == "unnest_clause" {
                child
                    .named_children(&mut child.walk())
                    .find(|c| c.kind() == "unnest_operator")
            } else {
                Some(child)
            };

            if let Some(root) = search_root {
                extract_and_mark_columns_from_node(
                    &root,
                    sql,
                    tables,
                    alias_map,
                    cte_columns,
                    graph,
                    true,
                );
            }
        }
    }
}

/// Process FROM clause in a CTE or final SELECT
fn process_from_clause(
    parent_node: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    for n in traverse(parent_node.walk(), Order::Pre) {
        if n.kind() == "from_clause" {
            // Process subqueries in FROM clause
            process_subqueries_in_from_clause(&n, sql, cte_columns, graph);

            // Process function calls and unnest clauses
            let (tables, alias_map) = extract_table(Some(n), sql);
            process_functions_and_unnest_in_from_clause(
                &n,
                sql,
                &tables,
                &alias_map,
                cte_columns,
                graph,
            );

            break; // Only process the first from_clause
        }
    }
}

/// Mark columns that are referenced in subqueries within FROM clauses
/// e.g., FROM unnest(generate_date_array(date((select min(col) from cte1)), ...))
fn mark_columns_used_in_from_clause_subqueries(
    node: &Node,
    sql: &str,
    graph: &mut HashMap<String, CTENode>,
) {
    let cte_columns: HashMap<String, Vec<ColumnInfo>> = graph
        .iter()
        .map(|(name, node)| (name.clone(), node.columns.clone()))
        .collect();

    for cte_node in find_ctes(node) {
        process_from_clause(&cte_node, sql, &cte_columns, graph);
    }

    if let Some(final_select) = find_final_select(node) {
        process_from_clause(&final_select, sql, &cte_columns, graph);
    }
}

/// Find from_clause sibling of a select_list node, skipping comments
fn find_from_clause_sibling<'a>(select_list_node: &Node<'a>) -> Option<Node<'a>> {
    let mut sibling = select_list_node.next_named_sibling();
    while let Some(s) = sibling {
        if s.kind() == "from_clause" {
            return Some(s);
        }
        sibling = s.next_named_sibling();
    }
    None
}

/// Check if a select_expression contains a window function
fn has_window_function(select_expr: &Node) -> bool {
    traverse(select_expr.walk(), Order::Pre).any(|c| c.kind() == "over_clause")
}

/// Process a select_expression in a CTE context
/// Returns true if the column was marked as used
fn process_select_expression_in_cte(
    select_expr: &Node,
    sql: &str,
    cte_name: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    let has_window_fn = has_window_function(select_expr);

    let mut col_refs = Vec::new();
    extract_column_references_from_expression(
        select_expr,
        sql,
        tables,
        alias_map,
        cte_columns,
        &mut col_refs,
    );

    for col_ref in col_refs {
        let col_name = extract_column_name(&col_ref.column_name);

        // For window functions, check if the column is defined in this CTE's SELECT list
        if has_window_fn {
            let exists_in_current_cte = cte_columns.get(cte_name).is_some_and(|current_cte_cols| {
                current_cte_cols
                    .iter()
                    .any(|c| extract_column_name(&c.column_name) == col_name)
            });

            if exists_in_current_cte {
                mark_column_as_used(graph, cte_name, col_name);
                continue;
            }
        }

        // Default: column from source table
        if let Some(table_name) = &col_ref.table_name {
            mark_column_as_used(graph, table_name, col_name);
        }
    }
}

/// Process a select_expression in final SELECT context
fn process_select_expression_in_final_select(
    select_expr: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    let mut col_refs = Vec::new();
    extract_column_references_from_expression(
        select_expr,
        sql,
        tables,
        alias_map,
        cte_columns,
        &mut col_refs,
    );

    for col_ref in col_refs {
        let col_name = extract_column_name(&col_ref.column_name);
        if let Some(table_name) = &col_ref.table_name {
            mark_column_as_used(graph, table_name, col_name);
        }
    }
}

/// Process all select_expressions in a select_list for a CTE
fn process_select_list_in_cte(
    select_list: &Node,
    sql: &str,
    cte_name: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    let from = find_from_clause_sibling(select_list);
    let (tables, alias_map) = extract_table(from, sql);

    for child in select_list.children(&mut select_list.walk()) {
        if child.kind() == "select_expression" {
            process_select_expression_in_cte(
                &child,
                sql,
                cte_name,
                &tables,
                &alias_map,
                cte_columns,
                graph,
            );
        }
    }
}

/// Process all select_expressions in a select_list for final SELECT
fn process_select_list_in_final_select(
    select_list: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    graph: &mut HashMap<String, CTENode>,
) {
    let from = find_from_clause_sibling(select_list);
    let (tables, alias_map) = extract_table(from, sql);

    for child in select_list.children(&mut select_list.walk()) {
        if child.kind() == "select_expression" {
            process_select_expression_in_final_select(
                &child,
                sql,
                &tables,
                &alias_map,
                cte_columns,
                graph,
            );
        }
    }
}

/// Mark columns that are referenced in SELECT expressions (e.g., in function arguments, window functions)
/// This ensures that columns used in complex expressions like sum(user_count) are marked as used
fn mark_columns_used_in_select_expressions(
    node: &Node,
    sql: &str,
    graph: &mut HashMap<String, CTENode>,
) {
    let cte_columns: HashMap<String, Vec<ColumnInfo>> = graph
        .iter()
        .map(|(name, node)| (name.clone(), node.columns.clone()))
        .collect();

    for cte_node in find_ctes(node) {
        let cte_name = get_cte_name(&cte_node, sql);

        for n in traverse(cte_node.walk(), Order::Pre) {
            if n.kind() == "select_list" {
                process_select_list_in_cte(&n, sql, &cte_name, &cte_columns, graph);
            }
        }
    }

    if let Some(final_select) = find_final_select(node) {
        for n in traverse(final_select.walk(), Order::Pre) {
            if n.kind() == "select_list" {
                process_select_list_in_final_select(&n, sql, &cte_columns, graph);
                break;
            }
        }
    }
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

/// Extract column name information when there is no alias
/// Returns (column_name, original_expression, source_column)
fn extract_column_name_without_alias(
    select_expr: &Node,
    sql: &str,
) -> (String, String, Option<String>) {
    let expr = select_expr.utf8_text(sql.as_bytes()).unwrap().to_string();
    let column_name = extract_column_name(&expr).to_string();
    let source = if column_name != expr {
        Some(column_name.clone())
    } else {
        None
    };
    (column_name, expr, source)
}

/// Extract alias information from an as_alias node
/// Returns (alias_name, original_expression, source_column)
fn extract_alias_info(
    alias_node: &Node,
    select_expr: &Node,
    sql: &str,
) -> (String, String, Option<String>) {
    // Extract alias name from as_alias node (last named child)
    let alias_name = alias_node
        .named_children(&mut alias_node.walk())
        .last()
        .map(|n| n.utf8_text(sql.as_bytes()).unwrap().to_string())
        .unwrap_or_default();

    // Extract original column (first named child of select_expression)
    let original = select_expr
        .named_child(0)
        .map(|n| n.utf8_text(sql.as_bytes()).unwrap().to_string())
        .unwrap_or_else(|| alias_name.clone());

    // source_column is the base column name without table prefix
    let source = extract_column_name(&original).to_string();
    (alias_name, original, Some(source))
}

/// Extract column information from a select_expression node
/// Returns ColumnInfo for the column defined by this expression
fn extract_column_info_from_select_expression(
    select_expr: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> ColumnInfo {
    // Check if there's an as_alias child
    let as_alias_node = select_expr
        .children(&mut select_expr.walk())
        .find(|n| n.kind() == "as_alias");

    // Extract column information (with or without alias)
    let (column, original_column, source_column) = as_alias_node.map_or_else(
        || extract_column_name_without_alias(select_expr, sql),
        |alias_node| extract_alias_info(&alias_node, select_expr, sql),
    );

    // Resolve the table for this column
    let table = find_original_table(&original_column, &tables.to_vec(), alias_map, cte_columns);

    ColumnInfo::new(
        Some(table),
        column,
        source_column,
        select_expr.start_position().row,
        select_expr.start_position().column,
    )
}

fn extract_columns(
    node: &Node,
    sql: &str,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> Vec<ColumnInfo> {
    let mut columns = Vec::new();

    if node.kind() == "select_list" {
        let from = node.next_named_sibling();
        let (tables, alias_map) = extract_table(from, sql);
        for child in node.children(&mut node.walk()) {
            if child.kind() == "select_expression" {
                let column_info = extract_column_info_from_select_expression(
                    &child,
                    sql,
                    &tables,
                    &alias_map,
                    cte_columns,
                );
                columns.push(column_info);
            } else if child.kind() == "select_all" {
                let position = child.start_position();
                columns.extend(expand_asterisk(position, &tables, cte_columns));
            }
        }
        // Return immediately after processing the first select_list
        // This prevents collecting columns from UNION or other subsequent SELECT clauses
        return columns;
    }

    for child in node.named_children(&mut node.walk()) {
        columns.extend(extract_columns(&child, sql, cte_columns));
    }

    columns
}

fn extract_tables_from_parent(node: &Node, sql: &str) -> (Vec<String>, HashMap<String, String>) {
    // Walk up to find the SELECT node, then get its FROM clause
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "select" {
            // Find the from_clause child of this SELECT
            for child in parent.named_children(&mut parent.walk()) {
                if child.kind() == "from_clause" {
                    return extract_table(Some(child), sql);
                }
            }
        }
        current = parent.parent();
    }
    (Vec::new(), HashMap::new())
}

fn extract_tables_from_qualify_parent(
    node: &Node,
    sql: &str,
) -> (Vec<String>, HashMap<String, String>) {
    // Walk up to find the SELECT node, then get its FROM clause
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "select" {
            // Find the from_clause child of this SELECT
            for child in parent.named_children(&mut parent.walk()) {
                if child.kind() == "from_clause" {
                    return extract_table(Some(child), sql);
                }
            }
        }
        current = parent.parent();
    }
    (Vec::new(), HashMap::new())
}

fn extract_columns_from_condition(
    node: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    columns: &mut Vec<ColumnInfo>,
) {
    // Extract the actual table names from potentially fully-qualified names
    // e.g., "`project`.`dataset`.`table`" -> "table"
    let actual_tables: Vec<String> = tables
        .iter()
        .map(|t| {
            // Split by '.' and take the last part
            let parts: Vec<&str> = t.split('.').collect();
            let last_part = parts.last().map_or(t.as_str(), |part| part);
            // Remove backticks if present
            last_part.trim_matches('`').to_string()
        })
        .collect();

    // Traverse the condition tree to find all column references
    for child in traverse(node.walk(), Order::Pre) {
        if child.kind() == "field" || child.kind() == "identifier" {
            let column_text = child.utf8_text(sql.as_bytes()).unwrap().to_string();

            // If column reference has a table prefix (e.g., "data1.id"), use that directly
            let table = if column_text.contains('.') {
                let prefix = column_text.split('.').next().unwrap_or("");
                // Check if prefix is an alias
                alias_map
                    .get(prefix)
                    .cloned()
                    .unwrap_or_else(|| prefix.to_string())
            } else {
                find_original_table(&column_text, &actual_tables, alias_map, cte_columns)
            };

            if !table.is_empty() {
                columns.push(ColumnInfo::new(
                    Some(table),
                    column_text,
                    None, // JOIN conditions don't have aliases
                    child.start_position().row,
                    child.start_position().column,
                ));
            }
        }
    }
}

/// Check if a node should be processed as a column reference
/// Returns true if the node is a "field" or an "identifier" that is not a function name
fn should_process_as_column_reference(node: &Node) -> bool {
    if node.kind() == "field" {
        return true;
    }

    if node.kind() == "identifier" {
        // Check if this identifier is NOT a function name
        let is_function_name = node.parent().is_some_and(|parent| {
            parent.kind() == "function_call"
                && parent
                    .child(0)
                    .is_some_and(|first_child| first_child.id() == node.id())
        });
        return !is_function_name;
    }

    false
}

/// Resolve the table name for a column reference
/// Handles both qualified (table.column) and unqualified columns
fn resolve_table_for_column(
    column_text: &str,
    actual_tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
) -> String {
    // If column reference has a table prefix (e.g., "data1.id"), use that directly
    if column_text.contains('.') {
        let prefix = column_text.split('.').next().unwrap_or("");
        // Check if prefix is an alias
        alias_map
            .get(prefix)
            .cloned()
            .unwrap_or_else(|| prefix.to_string())
    } else {
        // Unqualified column - find which table it belongs to
        find_original_table(column_text, &actual_tables.to_vec(), alias_map, cte_columns)
    }
}

/// Add a column reference to the columns list
fn add_column_reference(node: &Node, sql: &str, table: String, columns: &mut Vec<ColumnInfo>) {
    let column_text = node.utf8_text(sql.as_bytes()).unwrap().to_string();

    if !table.is_empty() {
        columns.push(ColumnInfo::new(
            Some(table),
            column_text,
            None,
            node.start_position().row,
            node.start_position().column,
        ));
    }
}

/// Extract the SELECT node from a query_expr or select node
/// Returns the select node if found, otherwise None
fn extract_select_node_from_query<'a>(node: &'a Node<'a>) -> Option<Node<'a>> {
    if node.kind() == "query_expr" {
        node.named_children(&mut node.walk())
            .find(|c| c.kind() == "select")
    } else if node.kind() == "select" {
        Some(*node)
    } else {
        None
    }
}

/// Extract tables and aliases from a subquery's context
/// If the subquery has no FROM clause, inherit parent's tables
fn extract_subquery_context(
    select_node: &Node,
    sql: &str,
    parent_tables: &[String],
    parent_alias_map: &HashMap<String, String>,
) -> (Vec<String>, HashMap<String, String>) {
    // Extract tables from this subquery's FROM clause
    let mut subquery_tables = Vec::new();
    let mut subquery_alias_map = HashMap::new();

    for child in select_node.named_children(&mut select_node.walk()) {
        if child.kind() == "from_clause" {
            (subquery_tables, subquery_alias_map) = extract_table(Some(child), sql);
            break;
        }
    }

    // If no FROM clause in subquery, use parent's tables
    if subquery_tables.is_empty() {
        subquery_tables = parent_tables.to_vec();
        subquery_alias_map = parent_alias_map.clone();
    }

    (subquery_tables, subquery_alias_map)
}

/// Process a subquery node and extract column references from it
/// Returns true if the subquery was successfully processed, false otherwise
fn process_subquery_node(
    node: &Node,
    sql: &str,
    parent_tables: &[String],
    parent_alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    columns: &mut Vec<ColumnInfo>,
) -> bool {
    // Extract the SELECT node from query_expr or select
    let select_node = match extract_select_node_from_query(node) {
        Some(sel) => sel,
        None => return false,
    };

    // Extract tables and aliases for this subquery
    let (subquery_tables, subquery_alias_map) =
        extract_subquery_context(&select_node, sql, parent_tables, parent_alias_map);

    // Recursively process the subquery
    extract_column_references_from_expression(
        &select_node,
        sql,
        &subquery_tables,
        &subquery_alias_map,
        cte_columns,
        columns,
    );

    true
}

/// Extract all column references from within a select_expression node
/// This includes columns used in function calls, window functions, etc.
/// Excludes identifiers that are part of as_alias nodes
fn extract_column_references_from_expression(
    node: &Node,
    sql: &str,
    tables: &[String],
    alias_map: &HashMap<String, String>,
    cte_columns: &HashMap<String, Vec<ColumnInfo>>,
    columns: &mut Vec<ColumnInfo>,
) {
    let actual_tables: Vec<String> = tables
        .iter()
        .filter(|t| !t.contains('.'))
        .cloned()
        .collect();

    // Traverse the expression tree to find all column references
    for child in traverse(node.walk(), Order::Pre) {
        // Skip as_alias nodes to avoid treating alias names as column references
        if child.kind() == "as_alias" {
            continue;
        }

        // Handle subqueries (scalar subqueries, subqueries in expressions)
        if (child.kind() == "select" || child.kind() == "query_expr")
            && process_subquery_node(&child, sql, tables, alias_map, cte_columns, columns)
        {
            continue;
        }

        // Process column references (both "field" and "identifier" nodes)
        if should_process_as_column_reference(&child) {
            let column_text = child.utf8_text(sql.as_bytes()).unwrap().to_string();
            let table =
                resolve_table_for_column(&column_text, &actual_tables, alias_map, cte_columns);
            add_column_reference(&child, sql, table, columns);
        }
    }
}

fn find_original_table(
    column: &str,
    tables: &Vec<String>,
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

fn extract_table(from: Option<Node>, sql: &str) -> (Vec<String>, HashMap<String, String>) {
    let mut tables = Vec::new();
    let mut alias_map = HashMap::new();

    if let Some(from_node) = from {
        for n in traverse(from_node.walk(), Order::Pre) {
            if n.kind() == "from_item" {
                // Get the first named child
                if let Some(first_child) = n.named_child(0) {
                    // Only process if it's an identifier (actual table name)
                    // Skip if it's a join_operation (which contains nested from_items)
                    if first_child.kind() == "identifier" {
                        let table_name = first_child.utf8_text(sql.as_bytes()).unwrap().to_string();
                        tables.push(table_name.clone());

                        // Check if there's an alias
                        for child in n.children(&mut n.walk()) {
                            if child.kind() == "as_alias" {
                                // Extract alias name (last named child of as_alias)
                                if let Some(alias_node) =
                                    child.named_children(&mut child.walk()).last()
                                {
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
        }
    }

    (tables, alias_map)
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

fn mark_column_as_used(graph: &mut HashMap<String, CTENode>, table_name: &str, column_name: &str) {
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
                // Use original_column_name if available (for aliased columns),
                // otherwise extract from current column_name
                let search_column_name = col_info.original_column_name.as_ref().map_or_else(
                    || extract_column_name(column_name).to_string(),
                    |original| original.clone(),
                );
                queue.push_back((actual_source_table.to_string(), search_column_name));
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
            // Need to check both exact match and base name match (e.g., "cmu.major" -> "major")
            let col_base_name = extract_column_name(&col.column_name);
            let is_used = cte_node.used_column_names.contains(&col.column_name)
                || cte_node
                    .used_column_names
                    .iter()
                    .any(|used| extract_column_name(used) == col_base_name);

            if !is_used {
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
    #[case("./sql/unused_column_in_cte_complex.sql", 5, vec!["unused_field1", "unused_field2", "unused_amount_field", "unused_price_field", "another_unused"])]
    #[case("./sql/unused_column_in_cte_join_only.sql", 1, vec!["unused_field"])]
    #[case("./sql/unused_column_in_cte_select_star_with_unused.sql", 2, vec!["unused_field1", "unused_field2"])]
    #[case("./sql/unused_column_in_cte_select_star_multiple_joins.sql", 1, vec!["id"])]
    #[case("./sql/unused_column_in_cte_column_alias.sql", 2, vec!["column2", "unused_column"])]
    #[case("./sql/unused_column_in_cte_multiple_alias.sql", 3, vec!["email", "unused_field1", "unused_field2"])]
    #[case("./sql/unused_column_in_cte_table_alias_without_as.sql", 2, vec!["id", "name"])]
    #[case("./sql/unused_column_in_cte_function_argument.sql", 1, vec!["unused_field"])]
    #[case("./sql/unused_column_in_cte_qualify.sql", 1, vec!["unused_field"])]
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
    #[case("./sql/unused_column_in_cte_select_star_from_join.sql")]
    #[case("./sql/unused_column_in_cte_table_alias.sql")]
    #[case("./sql/unused_column_in_cte_table_alias_join.sql")]
    #[case("./sql/unused_column_in_cte_select_star_chain.sql")]
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
        // Only SELECT * columns (JOIN conditions are handled separately now)
        assert_eq!(final_select_columns.len(), 3);

        // Check the columns (from SELECT *)
        let expected_columns_from_select = vec!["column1", "column2", "column3"];
        for (i, expected) in expected_columns_from_select.iter().enumerate() {
            assert_eq!("data3", final_select_columns[i].table_name.clone().unwrap());
            assert_eq!(*expected.to_string(), final_select_columns[i].column_name);
        }
    }
}
