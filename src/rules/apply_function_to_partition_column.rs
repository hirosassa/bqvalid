use crate::ast::NodeRef;

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::helpers::{get_node_text, one_based_start};
use crate::rules::rule::Rule;

const RULE_ID: &str = "apply_function_to_partition_column";

/// Date/time functions that, when wrapped around a partition column in a
/// filter, prevent BigQuery from pruning partitions. Partition columns are in
/// practice date/timestamp columns (or the `_PARTITIONTIME`/`_PARTITIONDATE`
/// pseudo columns), so limiting the rule to these functions keeps the signal
/// high: `WHERE UPPER(name) = 'X'` on a non-partition column is not flagged.
const DATE_TIME_FUNCTIONS: &[&str] = &[
    "date",
    "datetime",
    "timestamp",
    "time",
    "date_trunc",
    "datetime_trunc",
    "timestamp_trunc",
    "time_trunc",
    "format_date",
    "format_datetime",
    "format_timestamp",
    "format_time",
];

/// Cast target types that mark the cast as a date/time transform of the operand,
/// with the same partition-pruning consequences as the functions above.
const DATE_TIME_CAST_TYPES: &[&str] = &["date", "datetime", "timestamp", "time"];

/// Flags a date/time function or cast applied to a column in a WHERE-clause
/// comparison, which defeats partition pruning and forces a full scan.
pub struct ApplyFunctionToPartitionColumn;

impl Rule for ApplyFunctionToPartitionColumn {
    fn id(&self) -> &'static str {
        RULE_ID
    }

    fn check_node(&self, node: NodeRef<'_>, sql: &str, diagnostics: &mut Vec<Diagnostic>) {
        if node.kind() != "ASTWhereClause" {
            return;
        }
        for descendant in node.pre_order() {
            for operand in comparison_operands(&descendant) {
                if let Some(func) = date_time_transform_on_column(&operand, sql) {
                    diagnostics.push(new_full_scan_warning(&func));
                }
            }
        }
    }
}

/// The operands a comparison exposes to partition pruning, so both sides of a
/// binary comparison are checked (`date(col) = '...'` and `'...' = date(col)`).
///
/// - `ASTBinaryExpression`: the first and last children are the two operands
///   (any operator sits between them); either may carry the transform.
/// - `ASTBetweenExpression` / `ASTInExpression`: only the tested value (first
///   child) is a pruning candidate — wrapping the BETWEEN bounds or the IN list
///   values does not defeat pruning of that tested value.
///
/// Other node kinds expose nothing. Duplicate ids are dropped so a degenerate
/// single-operand node is never checked twice.
fn comparison_operands<'a>(node: &NodeRef<'a>) -> Vec<NodeRef<'a>> {
    let candidates = match node.kind() {
        "ASTBinaryExpression" => vec![node.child(0), node.children().into_iter().last()],
        "ASTBetweenExpression" | "ASTInExpression" => vec![node.child(0)],
        _ => return Vec::new(),
    };
    let mut operands: Vec<NodeRef<'a>> = Vec::new();
    for operand in candidates.into_iter().flatten() {
        if !operands.iter().any(|seen| seen.id() == operand.id()) {
            operands.push(operand);
        }
    }
    operands
}

/// Returns the offending `function_call` / `cast_expression` node when `operand`
/// is a date/time transform wrapped around a column reference, else `None`.
fn date_time_transform_on_column<'a>(operand: &NodeRef<'a>, sql: &str) -> Option<NodeRef<'a>> {
    match operand.kind() {
        "ASTFunctionCall" if is_date_time_function(operand, sql) && wraps_column(operand, sql) => {
            Some(*operand)
        }
        "ASTCastExpression" if casts_to_date_time(operand, sql) && wraps_column(operand, sql) => {
            Some(*operand)
        }
        // EXTRACT is inherently a date/time transform, so no function-name
        // allowlist check is needed.
        "ASTExtractExpression" if wraps_column(operand, sql) => Some(*operand),
        _ => None,
    }
}

/// True when the function-call node's name is one of [`DATE_TIME_FUNCTIONS`].
///
/// The name node is an `ASTPathExpression` (wrapping the identifier, same text).
fn is_date_time_function(func: &NodeRef<'_>, sql: &str) -> bool {
    func.named_child(0).is_some_and(|name| {
        name.kind() == "ASTPathExpression"
            && DATE_TIME_FUNCTIONS
                .iter()
                .any(|f| f.eq_ignore_ascii_case(get_node_text(&name, sql)))
    })
}

/// True when the cast's target type is one of [`DATE_TIME_CAST_TYPES`], e.g.
/// `cast(col as date)`. The type node is an `ASTSimpleType` on googlesql.
fn casts_to_date_time(cast: &NodeRef<'_>, sql: &str) -> bool {
    cast.named_children().into_iter().any(|child| {
        child.kind() == "ASTSimpleType"
            && DATE_TIME_CAST_TYPES
                .iter()
                .any(|t| t.eq_ignore_ascii_case(get_node_text(&child, sql)))
    })
}

/// True when the transform is applied to a column reference rather than only
/// literals or other column-free calls, e.g. `date(created_at)` (flagged) vs
/// `date('2024-01-01')` and `timestamp_trunc(current_timestamp(), day)` (not).
///
/// A column reference is an `ASTIdentifier` that is not one of the non-column
/// identifiers googlesql also models as identifiers, collected by
/// [`non_column_subtrees`]. Searching the whole subtree (not just the top-level
/// call) means a column nested under another call is still found.
fn wraps_column(operand: &NodeRef<'_>, sql: &str) -> bool {
    let nodes = operand.pre_order();
    let mut skip: Vec<usize> = Vec::new();
    for node in &nodes {
        skip.extend(non_column_subtrees(node, sql));
    }
    nodes.iter().any(|node| {
        node.kind() == "ASTIdentifier"
            && !skip.contains(&node.id())
            && !get_node_text(node, sql).is_empty()
    })
}

/// Ids of the identifiers under `node` that are not column references:
///
/// - a function call's name (`current_timestamp` in `current_timestamp()`);
/// - the date part argument of a `*_trunc` call (`day` in `timestamp_trunc(x, day)`);
/// - the date part of `EXTRACT(part FROM x)`;
/// - the unit of an `INTERVAL n unit` expression;
/// - a type name (`cast(col AS date)` wraps its own identifier on googlesql).
fn non_column_subtrees(node: &NodeRef<'_>, sql: &str) -> Vec<usize> {
    let excluded: Vec<NodeRef<'_>> = match node.kind() {
        "ASTFunctionCall" => {
            let mut parts: Vec<NodeRef<'_>> = node.named_child(0).into_iter().collect();
            if is_trunc_function(node, sql) {
                parts.extend(node.named_child(2));
            }
            parts
        }
        "ASTExtractExpression" => node.named_child(0).into_iter().collect(),
        "ASTIntervalExpr" => node
            .named_children()
            .into_iter()
            .filter(|child| child.kind() == "ASTIdentifier")
            .collect(),
        "ASTSimpleType" => vec![*node],
        _ => Vec::new(),
    };
    excluded
        .iter()
        .flat_map(|part| part.pre_order().into_iter().map(|n| n.id()))
        .collect()
}

/// True when the function-call node is a `*_trunc` call, whose third named child
/// (after the name and the value) is a date part keyword rather than a column.
fn is_trunc_function(func: &NodeRef<'_>, sql: &str) -> bool {
    func.named_child(0).is_some_and(|name| {
        get_node_text(&name, sql)
            .to_ascii_lowercase()
            .ends_with("_trunc")
    })
}

/// Build the full-scan diagnostic pointing at the offending transform node.
fn new_full_scan_warning(node: &NodeRef<'_>) -> Diagnostic {
    let (row, col) = one_based_start(node);
    Diagnostic::new(
        RULE_ID,
        Severity::Warning,
        row,
        col,
        "Full scan will cause! Should not apply a function to a partition column in a filter"
            .to_string(),
    )
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
    use crate::rules::helpers::run_rule;

    #[test]
    fn flags_date_function_on_column_in_binary_expression() {
        let sql = "select * from t where date(created_at) = '2024-01-01'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(diagnostics.len(), 1, "date(col) = ... must be flagged");
    }

    #[test]
    fn flags_cast_to_date_on_column() {
        let sql = "select * from t where cast(created_at as date) = '2024-01-01'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(diagnostics.len(), 1, "cast(col as date) must be flagged");
    }

    #[test]
    fn flags_date_function_on_column_on_right_operand() {
        // The transform can sit on either side of the comparison; a right-hand
        // `date(col)` defeats pruning just as a left-hand one does.
        let sql = "select * from t where '2024-01-01' = date(created_at)";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(diagnostics.len(), 1, "literal = date(col) must be flagged");
    }

    #[test]
    fn flags_cast_to_date_on_column_on_right_operand() {
        let sql = "select * from t where '2024-01-01' = cast(created_at as date)";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(
            diagnostics.len(),
            1,
            "literal = cast(col as date) must be flagged"
        );
    }

    #[test]
    fn flags_trunc_function_on_column() {
        let sql = "select * from t where timestamp_trunc(ts, day) = '2024-01-01'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(
            diagnostics.len(),
            1,
            "timestamp_trunc(col, ..) must be flagged"
        );
    }

    #[test]
    fn flags_function_in_between_expression() {
        let sql = "select * from t where date(created_at) between '2024-01-01' and '2024-01-02'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(diagnostics.len(), 1, "date(col) between .. must be flagged");
    }

    #[test]
    fn flags_pseudo_partition_column_wrapped_in_function() {
        let sql = "select * from t where date(_partitiontime) = '2024-01-01'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(diagnostics.len(), 1, "date(_partitiontime) must be flagged");
    }

    #[test]
    fn flags_date_function_on_column_in_in_expression() {
        // `date(col) IN (...)` wraps the tested value in a transform and defeats
        // pruning just like a binary comparison does.
        let sql = "select * from t where date(created_at) in ('2024-01-01', '2024-01-02')";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(diagnostics.len(), 1, "date(col) IN (..) must be flagged");
    }

    #[test]
    fn does_not_flag_bare_column_in_expression() {
        let sql = "select * from t where created_at in ('2024-01-01', '2024-01-02')";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert!(
            diagnostics.is_empty(),
            "a bare column IN list prunes partitions and must not be flagged"
        );
    }

    #[test]
    fn flags_extract_on_column() {
        let sql = "select * from t where extract(year from created_at) = 2024";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(diagnostics.len(), 1, "extract(.. from col) must be flagged");
    }

    #[test]
    fn does_not_flag_extract_on_literal() {
        // The date part (`year`) must not be mistaken for a column: with a literal
        // operand there is no partition column to prune.
        let sql = "select * from t where extract(year from date '2024-01-01') = 2024";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert!(
            diagnostics.is_empty(),
            "extract from a literal has no column to prune"
        );
    }

    #[test]
    fn flags_format_date_on_column() {
        let sql = "select * from t where format_date('%Y-%m', created_at) = '2024-01'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(
            diagnostics.len(),
            1,
            "format_date(fmt, col) must be flagged"
        );
    }

    #[test]
    fn flags_format_timestamp_on_column() {
        let sql = "select * from t where format_timestamp('%Y', created_at) = '2024'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(
            diagnostics.len(),
            1,
            "format_timestamp(fmt, col) must be flagged"
        );
    }

    #[test]
    fn does_not_flag_bare_column_comparison() {
        let sql = "select * from t where created_at >= '2024-01-01'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert!(
            diagnostics.is_empty(),
            "a bare column comparison prunes partitions and must not be flagged"
        );
    }

    #[test]
    fn does_not_flag_non_date_function() {
        let sql = "select * from t where upper(name) = 'FOO'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert!(
            diagnostics.is_empty(),
            "non-date functions are unrelated to partition pruning"
        );
    }

    #[test]
    fn does_not_flag_date_function_on_literal_only() {
        // No column reference inside, so there is no partition column to prune.
        let sql = "select * from t where created_at >= date('2024-01-01')";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert!(
            diagnostics.is_empty(),
            "date(literal) does not defeat pruning of a bare column"
        );
    }

    #[test]
    fn does_not_flag_constant_function_call_on_comparison_rhs() {
        // `TIMESTAMP_TRUNC(CURRENT_TIMESTAMP(), DAY)` takes no column: the inner
        // function name and the `DAY` date part must not be read as columns. This
        // is the recommended pruning-friendly pattern.
        let sql = "SELECT x FROM t WHERE ts2 > TIMESTAMP_TRUNC(CURRENT_TIMESTAMP(), DAY)";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert!(
            diagnostics.is_empty(),
            "a column-free date/time call must not be flagged ({} found)",
            diagnostics.len()
        );
    }

    #[test]
    fn does_not_flag_other_constant_date_time_calls() {
        for sql in [
            "SELECT x FROM t WHERE d >= DATE_TRUNC(CURRENT_DATE(), MONTH)",
            "SELECT x FROM t WHERE ts >= TIMESTAMP(DATE_SUB(CURRENT_DATE(), INTERVAL 7 DAY))",
            "SELECT x FROM t WHERE d = DATE(CURRENT_TIMESTAMP(), 'Asia/Tokyo')",
        ] {
            let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
            assert!(
                diagnostics.is_empty(),
                "constant call must not be flagged: {sql} ({} found)",
                diagnostics.len()
            );
        }
    }

    #[test]
    fn flags_trunc_with_nested_function_around_column() {
        // A column nested under another function call still defeats pruning.
        let sql = "SELECT x FROM t WHERE TIMESTAMP_TRUNC(TIMESTAMP_ADD(ts, INTERVAL 1 HOUR), DAY) = '2024-01-01'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(diagnostics.len(), 1, "nested column must still be flagged");
    }

    #[test]
    fn does_not_flag_cast_to_string() {
        // Only date/time cast targets are treated as pruning-defeating transforms.
        let sql = "select * from t where cast(name as string) = 'x'";
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert!(diagnostics.is_empty(), "cast to string is out of scope");
    }

    #[test]
    fn points_at_the_function_position() {
        let sql = "SELECT x FROM t WHERE DATE(created_at) = '2024-01-01'";
        let func_col = sql.find("DATE").expect("query contains DATE(");
        let diagnostics = run_rule(&ApplyFunctionToPartitionColumn, sql);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].row(), 1);
        assert_eq!(diagnostics[0].col(), func_col + 1);
    }

    #[test]
    fn does_not_panic_on_non_comparison_where_shapes() {
        // WHERE clauses that are not a simple binary/between comparison must not
        // panic the node navigation (and must not be flagged).
        for sql in [
            "SELECT x FROM t WHERE is_active",
            "SELECT x FROM t WHERE created_at IN ('2024-01-01', '2024-01-02')",
            "SELECT x FROM t WHERE DATE(created_at) IS NOT NULL",
        ] {
            let _ = run_rule(&ApplyFunctionToPartitionColumn, sql);
        }
    }
}
