use clap::Parser;
use std::fmt::Display;
use std::fs;
use std::io::{self, Read};
use std::process::ExitCode;
use tree_sitter::Node;
use tree_sitter::Parser as TsParser;
use tree_sitter_sql_bigquery::language;
use tree_sitter_traversal::{traverse, Order};
use walkdir::{DirEntry, WalkDir};

#[derive(Debug, Parser)]
#[clap(
    name = env!("CARGO_PKG_NAME"),
    author = env!("CARGO_PKG_AUTHORS"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    version = env!("CARGO_PKG_VERSION"),
    arg_required_else_help = true,
)]
struct Args {
    files: Vec<String>,
}

fn main() -> ExitCode {
    let stdin = io::stdin();
    let args = Args::parse();

    // stdin
    if args.files.is_empty() {
        if let Some(diagnostics) = analyse_sql(&mut stdin.lock()) {
            for diagnostic in diagnostics {
                eprintln!("{}", diagnostic);
            }
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    // files
    let targets = args.files.into_iter().flat_map(|f| {
        WalkDir::new(f)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(is_sql)
    });

    let mut all_diagnostics = Vec::new();

    for target in targets {
        let file_path = target.into_path();
        if let Ok(mut file) = fs::File::open(&file_path) {
            if let Some(diagnostics) = analyse_sql(&mut file) {
                for diagnostic in diagnostics {
                    eprintln!("{}: {}", file_path.display(), diagnostic);
                    all_diagnostics.push(diagnostic);
                }
            }
        }
    }

    if !all_diagnostics.is_empty() {
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

fn is_sql(entry: &DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| s.ends_with(".sql"))
        .unwrap_or(false)
}

/// Represents a diagnostic, such as a full scan error.
///
/// rows and columns are 1-based.
struct Diagnostic {
    row: usize,
    col: usize,
    message: String,
}

impl Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.row, self.col, self.message)
    }
}

fn new_current_date_warning(row: usize, col: usize) -> Diagnostic {
    Diagnostic {
        row: row + 1,
        col: col + 1,
        message: "CURRENT_DATE is used!".to_string(),
    }
}

fn new_full_scan_warning(row: usize, col: usize) -> Diagnostic {
    Diagnostic {
        row: row + 1,
        col: col + 1,
        message: "Full scan will cause! Should not compare _TABLE_SUFFIX with subquery".to_string(),
    }
}

fn analyse_sql<F: Read>(f: &mut F) -> Option<Vec<Diagnostic>> {
    let mut sql = String::new();
    let _ = f.read_to_string(&mut sql);

    let mut parser = TsParser::new();
    parser.set_language(language()).unwrap();
    let tree = parser.parse(&sql, None).unwrap();

    let mut diagnostics = Vec::new();

    for node in traverse(tree.walk(), Order::Pre) {
        if let Some(diagnostic) = current_date_used(node, &sql) {
            diagnostics.push(diagnostic);
        }

        if node.kind() == "where_clause" {
            if let Some(diagnostic) = compared_with_subquery_in_binary_expression(node, &sql) {
                diagnostics.push(diagnostic);
            }
            if let Some(diagnostic) = compared_with_subquery_in_between_expression(node, &sql) {
                diagnostics.push(diagnostic);
            }
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

fn compared_with_subquery_in_binary_expression(n: Node, src: &str) -> Option<Diagnostic> {
    for node in traverse(n.walk(), Order::Pre) {
        let range = node.range();
        let text = &src[range.start_byte..range.end_byte];

        if node.kind() == "identifier" && text.to_ascii_lowercase() == "_table_suffix" {
            let parent = node.parent().unwrap();
            let mut tc = parent.walk();
            let right_operand = parent.children(&mut tc).last().unwrap();
            if parent.kind() == "binary_expression"
                && right_operand.kind() == "select_subexpression"
            {
                let rg = right_operand.range();
                return Some(new_full_scan_warning(
                    rg.start_point.row,
                    rg.start_point.column,
                ));
            }
        }
    }
    None
}

fn compared_with_subquery_in_between_expression(n: Node, src: &str) -> Option<Diagnostic> {
    for node in traverse(n.walk(), Order::Pre) {
        let range = node.range();
        let text = &src[range.start_byte..range.end_byte];

        if node.kind() == "identifier" && text.to_ascii_lowercase() == "_table_suffix" {
            let parent = node.parent().unwrap();
            if parent.kind() == "between_operator" {
                let mut tc = parent.walk();
                for c in parent.children(&mut tc) {
                    if (c.kind() == "between_from" || c.kind() == "between_to")
                        && c.child(0).unwrap().kind() == "select_subexpression"
                    {
                        let rg = c.child(0).unwrap().range();
                        return Some(new_full_scan_warning(
                            rg.start_point.row,
                            rg.start_point.column,
                        ));
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::tempdir;

    #[test]
    fn valid() {
        let mut parser = TsParser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/valid.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert_eq!(
                    compared_with_subquery_in_binary_expression(node, &sql).is_some(),
                    false
                );
                assert_eq!(
                    compared_with_subquery_in_between_expression(node, &sql).is_some(),
                    false
                );
            }
        }
    }

    #[test]
    fn binary_op() {
        let mut parser = TsParser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/subquery_with_binary_op.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_binary_expression(node, &sql).is_some());
            }
        }
    }

    #[test]
    fn between_from() {
        let mut parser = TsParser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/subquery_with_between_from.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_between_expression(node, &sql).is_some());
            }
        }
    }

    #[test]
    fn between_to() {
        let mut parser = TsParser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/subquery_with_between_to.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        for node in traverse(tree.walk(), Order::Pre) {
            if node.kind() == "where_clause" {
                assert!(compared_with_subquery_in_between_expression(node, &sql).is_some());
            }
        }
    }

    #[test]
    fn current_date_is_used() {
        let mut parser = TsParser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/current_date_is_used.sql").unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        let mut ds = Vec::new();
        for node in traverse(tree.walk(), Order::Pre) {
            if let Some(diag) = current_date_used(node, &sql) {
                ds.push(diag);
            }
        }
        assert!(ds.len() > 0);
    }

    #[test]
    fn current_date_is_not_used() {
        let mut parser = TsParser::new();
        parser.set_language(language()).unwrap();

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

    #[test]
    fn test_is_sql_true() {
        let filename = "sample.sql";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join(filename);
        let _ = File::create(&file_path).unwrap();

        for e in WalkDir::new(file_path).into_iter().filter_map(|e| e.ok()) {
            assert!(is_sql(&e));
        }
    }

    #[test]
    fn test_is_sql_false() {
        let filename = "sample.txt";
        let dir = tempdir().unwrap();
        let file_path = dir.path().join(filename);
        let _ = File::create(&file_path).unwrap();

        for e in WalkDir::new(file_path).into_iter().filter_map(|e| e.ok()) {
            assert!(!is_sql(&e));
        }
    }

    #[test]
    fn multiple_messages_in_single_sql_file() {
        let mut parser = TsParser::new();
        parser.set_language(language()).unwrap();

        let sql = fs::read_to_string("./sql/current_date_and_subquery_with_between_are_used.sql")
            .unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        let mut diagnostics = Vec::new();
        for node in traverse(tree.walk(), Order::Pre) {
            if let Some(diagnostic) = current_date_used(node, &sql) {
                diagnostics.push(diagnostic);
            }
            if node.kind() == "where_clause" {
                if let Some(diagnostic) = compared_with_subquery_in_binary_expression(node, &sql) {
                    diagnostics.push(diagnostic);
                }
                if let Some(diagnostic) = compared_with_subquery_in_between_expression(node, &sql) {
                    diagnostics.push(diagnostic);
                }
            }
        }
        assert!(diagnostics.len() > 1);
    }
}
