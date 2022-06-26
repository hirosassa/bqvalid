use clap::Parser;
use std::collections::HashMap;
use std::fmt::Display;
use std::fs;
use std::io::{self, Read};
use std::process::ExitCode;
use tree_sitter::Node;
use tree_sitter::Parser as TsParser;
use tree_sitter_sql_bigquery::language;
use tree_sitter_traversal::{traverse, Order};
use walkdir::WalkDir;

#[derive(Debug, Parser)]
#[clap(
    name = env!("CARGO_PKG_NAME"),
    author=env!("CARGO_PKG_AUTHORS"),
    about=env!("CARGO_PKG_DESCRIPTION"),
    version=env!("CARGO_PKG_VERSION"),
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
        if let Some(diagnostic) = analyse_sql(&mut stdin.lock()) {
            eprintln!("{}", diagnostic);
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    }

    // files
    let errors: HashMap<String, Option<Diagnostic>> = args
        .files
        .into_iter()
        .flat_map(|f| {
            WalkDir::new(f)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .map(|e| e.ends_with(".sql"))
                        .unwrap_or(false)
                })
                .map(|f| -> (String, Option<Diagnostic>) {
                    let key = f.into_path();
                    (
                        key.to_str().unwrap_or("").to_string(),
                        fs::File::open(key)
                            .map(|ref mut file| analyse_sql(file))
                            .expect("failed to open file"),
                    )
                })
        })
        .filter(|(_k, v)| v.is_some())
        .collect();
    if !errors.is_empty() {
        for (k, v) in errors.iter() {
            if let Some(v) = v {
                eprintln!("{}:{}", k, v);
            }
        }
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

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

fn new_full_scan_warning(row: usize, col: usize) -> Diagnostic {
    Diagnostic {
        row,
        col,
        message: "Full scan will cause! Should not compare _TABLE_SUFFIX with subquery".to_string(),
    }
}

fn analyse_sql<F: Read>(f: &mut F) -> Option<Diagnostic> {
    let mut sql = String::new();
    let _ = f.read_to_string(&mut sql);

    let mut parser = TsParser::new();
    parser.set_language(language()).unwrap();
    let tree = parser.parse(&sql, None).unwrap();

    for node in traverse(tree.walk(), Order::Pre) {
        if node.kind() == "where_clause" {
            if let Some(diagnostic) = compared_with_subquery_in_binary_expression(node, &sql) {
                return Some(diagnostic);
            }
            if let Some(diagnostic) = compared_with_subquery_in_between_expression(node, &sql) {
                return Some(diagnostic);
            }
        }
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
    use std::fs;

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
}
