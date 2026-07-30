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
        if node.kind() != "where_clause" {
            return;
        }
        for descendant in node.pre_order() {
            let operand = match descendant.kind() {
                // A comparison's left operand is the first child of the
                // binary/between node (e.g. `date(col) = '...'`,
                // `date(col) between '...' and '...'`).
                "binary_expression" | "between_operator" => descendant.child(0),
                _ => continue,
            };
            let Some(operand) = operand else {
                continue;
            };
            if let Some(func) = date_time_transform_on_column(&operand, sql) {
                diagnostics.push(new_full_scan_warning(&func));
            }
        }
    }
}

/// Returns the offending `function_call` / `cast_expression` node when `operand`
/// is a date/time transform wrapped around a column reference, else `None`.
fn date_time_transform_on_column<'a>(operand: &NodeRef<'a>, sql: &str) -> Option<NodeRef<'a>> {
    match operand.kind() {
        // `function_call`'s first named child is the function name (also an
        // `identifier`), so skip it when looking for a wrapped column.
        "function_call"
            if is_date_time_function(operand, sql) && wraps_column(operand, sql, true) =>
        {
            Some(*operand)
        }
        // `cast_expression`'s first named child is the operand itself, so keep it.
        "cast_expression"
            if casts_to_date_time(operand, sql) && wraps_column(operand, sql, false) =>
        {
            Some(*operand)
        }
        _ => None,
    }
}

/// True when the `function_call`'s name is one of [`DATE_TIME_FUNCTIONS`].
fn is_date_time_function(func: &NodeRef<'_>, sql: &str) -> bool {
    func.named_child(0).is_some_and(|name| {
        name.kind() == "identifier"
            && DATE_TIME_FUNCTIONS
                .iter()
                .any(|f| f.eq_ignore_ascii_case(get_node_text(&name, sql)))
    })
}

/// True when the `cast_expression`'s target type is one of
/// [`DATE_TIME_CAST_TYPES`], e.g. `cast(col as date)`.
fn casts_to_date_time(cast: &NodeRef<'_>, sql: &str) -> bool {
    cast.named_children().into_iter().any(|child| {
        child.kind() == "type_identifier"
            && DATE_TIME_CAST_TYPES
                .iter()
                .any(|t| t.eq_ignore_ascii_case(get_node_text(&child, sql)))
    })
}

/// True when the transform is applied to a column reference rather than only
/// literals, e.g. `date(created_at)` (flagged) vs `date('2024-01-01')` (not).
///
/// The transform's own name is an `identifier` too, so it is skipped: a column
/// reference is an `identifier` that is not the function name.
fn wraps_column(operand: &NodeRef<'_>, sql: &str, skip_first_named_child: bool) -> bool {
    let skip_id = if skip_first_named_child {
        operand.named_child(0).map(|n| n.id())
    } else {
        None
    };
    operand.pre_order().into_iter().any(|node| {
        node.kind() == "identifier"
            && Some(node.id()) != skip_id
            && !get_node_text(&node, sql).is_empty()
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
    fn does_not_panic_on_malformed_where_clause() {
        for sql in [
            "SELECT x FROM t WHERE DATE(",
            "SELECT x FROM t WHERE DATE(created_at) =",
            "WHERE DATE(created_at)",
        ] {
            let _ = run_rule(&ApplyFunctionToPartitionColumn, sql);
        }
    }
}
