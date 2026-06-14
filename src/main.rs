use bqvalid::diagnostic::Diagnostic;
use bqvalid::rules::compare_table_suffix_with_subquery;
use bqvalid::rules::invalid_group_by;
use bqvalid::rules::unnecessary_order_by;
use bqvalid::rules::unused_column_in_cte;
use bqvalid::rules::use_current_date;
use clap::Parser;
use clap_verbosity_flag::Verbosity;
use log::debug;
use std::fs;
use std::io::{self, Read};
use std::process::ExitCode;
use tree_sitter::Parser as TsParser;
use tree_sitter_sql_bigquery::language;
use walkdir::{DirEntry, WalkDir};

fn get_version() -> &'static str {
    option_env!("CARGO_PKG_VERSION")
        .filter(|&v| v != "0.0.0")
        .or(option_env!("BUILD_VERSION"))
        .unwrap_or("unknown")
}

#[derive(Debug, Parser)]
#[clap(
    name = env!("CARGO_PKG_NAME"),
    author = env!("CARGO_PKG_AUTHORS"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    version = get_version(),
)]
struct Args {
    files: Vec<String>,

    #[clap(flatten)]
    verbose: Verbosity,
}

fn main() -> ExitCode {
    let stdin = io::stdin();
    let args = Args::parse();
    env_logger::Builder::new()
        .filter_level(args.verbose.log_level_filter())
        .init();
    debug!("verbose mode");

    // stdin
    if args.files.is_empty() {
        let mut sql = String::new();
        let read_result = stdin.lock().read_to_string(&mut sql);
        if let Err(e) = read_result {
            eprintln!("Error reading stdin: {}", e);
            return ExitCode::FAILURE;
        }
        let diagnostics = analyse_sql(&sql);
        for diagnostic in &diagnostics {
            eprintln!("{}", diagnostic);
        }
        return if diagnostics.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
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
        match fs::read_to_string(&file_path) {
            Ok(sql) => {
                let diagnostics = analyse_sql(&sql);
                for diagnostic in &diagnostics {
                    eprintln!("{}: {}", file_path.display(), diagnostic);
                }
                all_diagnostics.extend(diagnostics);
            }
            Err(e) => {
                eprintln!("{}: Error reading file: {}", file_path.display(), e);
            }
        }
    }

    if all_diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn is_sql(entry: &DirEntry) -> bool {
    entry
        .path()
        .extension()
        .map(|s| s == "sql")
        .unwrap_or(false)
}

fn analyse_sql(sql: &str) -> Vec<Diagnostic> {
    let mut parser = TsParser::new();
    parser.set_language(&language()).unwrap();
    let tree = parser.parse(sql, None).unwrap();

    let mut diagnostics = Vec::new();
    diagnostics.extend(compare_table_suffix_with_subquery::check(&tree, sql));
    diagnostics.extend(invalid_group_by::check(&tree, sql));
    diagnostics.extend(unnecessary_order_by::check(&tree, sql));
    diagnostics.extend(unused_column_in_cte::check(&tree, sql));
    diagnostics.extend(use_current_date::check(&tree, sql));
    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::tempdir;

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
        parser.set_language(&language()).unwrap();

        let sql = fs::read_to_string("./sql/current_date_and_subquery_with_between_are_used.sql")
            .unwrap();
        let tree = parser.parse(&sql, None).unwrap();

        let mut diagnostics = Vec::new();
        diagnostics.extend(compare_table_suffix_with_subquery::check(&tree, &sql));
        diagnostics.extend(use_current_date::check(&tree, &sql));
        assert!(diagnostics.len() > 1);
    }
}
