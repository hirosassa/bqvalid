use tree_sitter::Node;

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::helpers::{get_node_text, one_based_start};
use crate::rules::rule::Rule;

const RULE_ID: &str = "use_current_date";

/// Flags `CURRENT_DATE`, which hurts query reproducibility.
pub struct UseCurrentDate;

impl Rule for UseCurrentDate {
    fn id(&self) -> &'static str {
        RULE_ID
    }

    fn check_node(&self, node: Node<'_>, sql: &str, diagnostics: &mut Vec<Diagnostic>) {
        if let Some(diagnostic) = current_date_used(node, sql) {
            diagnostics.push(diagnostic);
        }
    }
}

fn current_date_used(node: Node, src: &str) -> Option<Diagnostic> {
    let text = get_node_text(&node, src);

    if node.kind() == "identifier" && text.eq_ignore_ascii_case("current_date") {
        let (row, col) = one_based_start(&node);
        return Some(Diagnostic::new(
            RULE_ID,
            Severity::Warning,
            row,
            col,
            "CURRENT_DATE is used!".to_string(),
        ));
    }
    None
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
    fn current_date_is_used() {
        let sql = "\
select
  current_date,
  column_a
from
  dataset.table
";
        assert!(!run_rule(&UseCurrentDate, sql).is_empty());
    }

    #[test]
    fn current_date_is_not_used() {
        let sql = "\
select
  *
from
  dataset.table
";
        assert!(run_rule(&UseCurrentDate, sql).is_empty());
    }

    #[test]
    fn check_flags_every_occurrence() {
        // Two calls on one line -> two diagnostics, each pointing at its own column.
        let sql = "SELECT CURRENT_DATE(), CURRENT_DATE() FROM t";

        let diagnostics = run_rule(&UseCurrentDate, sql);
        assert_eq!(diagnostics.len(), 2);
        assert!(diagnostics.iter().all(|d| d.row() == 1));

        let expected_cols: Vec<usize> = sql
            .match_indices("CURRENT_DATE")
            .map(|(i, _)| i + 1)
            .collect();
        assert_eq!(expected_cols.len(), 2);
        let cols: Vec<usize> = diagnostics.iter().map(Diagnostic::col).collect();
        for expected in expected_cols {
            assert!(
                cols.contains(&expected),
                "missing diagnostic at col {expected}"
            );
        }
    }

    #[test]
    fn check_is_case_insensitive() {
        // Lowercase spelling must be flagged just like the canonical uppercase.
        let sql = "SELECT current_date() FROM t";
        assert_eq!(run_rule(&UseCurrentDate, sql).len(), 1);
    }
}
