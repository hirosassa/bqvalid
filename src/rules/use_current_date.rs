use tree_sitter::Node;

use crate::diagnostic::Diagnostic;
use crate::rules::helpers::get_node_text;
use crate::rules::rule::Rule;

/// Flags `CURRENT_DATE`, which hurts query reproducibility.
pub struct UseCurrentDate;

impl Rule for UseCurrentDate {
    fn id(&self) -> &'static str {
        "use_current_date"
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
        return Some(new_current_date_warning(
            node.range().start_point.row,
            node.range().start_point.column,
        ));
    }
    None
}

fn new_current_date_warning(row: usize, col: usize) -> Diagnostic {
    Diagnostic::new(
        row.saturating_add(1),
        col.saturating_add(1),
        "CURRENT_DATE is used!".to_string(),
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
    use crate::rules::helpers::parse_sql;
    use std::fs;

    #[test]
    fn current_date_is_used() {
        let sql = fs::read_to_string("./sql/current_date_is_used.sql").unwrap();
        let tree = parse_sql(&sql);

        assert!(!UseCurrentDate.check(&tree, &sql).is_empty());
    }

    #[test]
    fn current_date_is_not_used() {
        let sql = fs::read_to_string("./sql/sample.sql").unwrap();
        let tree = parse_sql(&sql);

        assert!(UseCurrentDate.check(&tree, &sql).is_empty());
    }

    #[test]
    fn check_flags_every_occurrence() {
        // Two calls on one line -> two diagnostics, each pointing at its own column.
        let sql = "SELECT CURRENT_DATE(), CURRENT_DATE() FROM t";
        let tree = parse_sql(sql);

        let diagnostics = UseCurrentDate.check(&tree, sql);
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
        let tree = parse_sql(sql);
        assert_eq!(UseCurrentDate.check(&tree, sql).len(), 1);
    }
}
