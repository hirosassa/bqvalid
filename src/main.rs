use bqvalid::diagnostic::Diagnostic;
use bqvalid::rules::compare_table_suffix_with_subquery;
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

#[derive(Debug, Parser)]
#[clap(
    name = env!("CARGO_PKG_NAME"),
    author = env!("CARGO_PKG_AUTHORS"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    version = env!("CARGO_PKG_VERSION"),
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
        let mut handle = stdin.lock();
        if let Some(diagnostics) = analyse_sql(&mut handle) {
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

    #[allow(clippy::collapsible_if)]
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
        .path()
        .extension()
        .map(|s| s == "sql")
        .unwrap_or(false)
}

fn analyse_sql<F: Read>(f: &mut F) -> Option<Vec<Diagnostic>> {
    let mut sql = String::new();
    let _ = f.read_to_string(&mut sql);

    let mut parser = TsParser::new();
    parser.set_language(&language()).unwrap();
    let tree = parser.parse(&sql, None).unwrap();

    let mut diagnostics = Vec::new();

    if let Some(diags) = compare_table_suffix_with_subquery::check(&tree, &sql) {
        diagnostics.extend(diags);
    }

    if let Some(diags) = unnecessary_order_by::check(&tree, &sql) {
        diagnostics.extend(diags);
    }

    if let Some(diags) = unused_column_in_cte::check(&tree, &sql) {
        diagnostics.extend(diags);
    }

    if let Some(diags) = use_current_date::check(&tree, &sql) {
        diagnostics.extend(diags);
    }

    if diagnostics.is_empty() {
        None
    } else {
        Some(diagnostics)
    }
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
        if let Some(diags) = compare_table_suffix_with_subquery::check(&tree, &sql) {
            diagnostics.extend(diags);
        }

        if let Some(diags) = use_current_date::check(&tree, &sql) {
            diagnostics.extend(diags);
        }
        assert!(diagnostics.len() > 1);
    }
}
