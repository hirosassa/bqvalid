use tree_sitter::{Node, Tree};
use tree_sitter_traversal::{traverse, Order};

use crate::diagnostic::Diagnostic;

pub fn check(tree: &Tree, sql: &str) -> Option<Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();

    for node in traverse(tree.walk(), Order::Pre) {
        if let Some(diagnostic) = current_date_used(node, sql) {
            diagnostics.push(diagnostic);
        }
    }

    if diagnostics.is_empty() {
        None
    } else {
        Some(diagnostics)
    }
}

fn current_date_used(node: Node, src: &str) -> Option<Diagnostic> {
    let range = node.range();
    let text = &src[range.start_byte..range.end_byte];

    if node.kind() == "identifier" && text.to_ascii_lowercase() == "current_date" {
        return Some(new_current_date_warning(
            range.start_point.row,
            range.start_point.column,
        ));
    }
    None
}

fn new_current_date_warning(row: usize, col: usize) -> Diagnostic {
    Diagnostic::new(row + 1, col + 1, "CURRENT_DATE is used!".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tree_sitter::Parser as TsParser;
    use tree_sitter_sql_bigquery::language;

    #[test]
    fn current_date_is_used() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/current_date_is_used.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        let mut ds = Vec::new();
        for node in traverse(tree.walk(), Order::Pre) {
            if let Some(diag) = current_date_used(node, &sql) {
                ds.push(diag);
            }
        }
        assert!(!ds.is_empty());
    }

    #[test]
    fn current_date_is_not_used() {
        let mut parser = TsParser::new();
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/sample.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        let mut ds = Vec::new();
        for node in traverse(tree.walk(), Order::Pre) {
            if let Some(diag) = current_date_used(node, &sql) {
                ds.push(diag);
            }
        }
        assert!(ds.is_empty());
    }
}
