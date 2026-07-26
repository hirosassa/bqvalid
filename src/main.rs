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
            .filter_map(|e| match e {
                Ok(entry) => Some(entry),
                Err(err) => {
                    eprintln!("Error walking path: {}", err);
                    None
                }
            })
            .filter(is_sql)
    });

    let mut all_diagnostics = Vec::new();
    let mut has_error = false;

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
                has_error = true;
            }
        }
    }

    if has_error || !all_diagnostics.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
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
    if let Err(e) = parser.set_language(&language()) {
        eprintln!("Error loading BigQuery grammar: {}", e);
        return Vec::new();
    }
    let Some(tree) = parser.parse(sql, None) else {
        eprintln!("Error parsing SQL input");
        return Vec::new();
    };

    let mut diagnostics = Vec::new();
    diagnostics.extend(compare_table_suffix_with_subquery::check(&tree, sql));
    diagnostics.extend(invalid_group_by::check(&tree, sql));
    diagnostics.extend(unnecessary_order_by::check(&tree, sql));
    diagnostics.extend(unused_column_in_cte::check(&tree, sql));
    diagnostics.extend(use_current_date::check(&tree, sql));
    diagnostics
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

    #[test]
    fn analyse_sql_empty_input_yields_no_diagnostics() {
        assert!(analyse_sql("").is_empty());
    }

    #[test]
    fn analyse_sql_clean_query_yields_no_diagnostics() {
        // A plain, well-formed query with none of the linted anti-patterns.
        assert!(analyse_sql("SELECT id, name FROM users").is_empty());
    }

    #[test]
    fn analyse_sql_aggregates_multiple_rules_from_a_single_query() {
        // Triggers both use_current_date and compare_table_suffix_with_subquery,
        // proving analyse_sql fans a query out across every rule and merges results.
        let sql = "SELECT CURRENT_DATE() AS d \
                   FROM t \
                   WHERE _TABLE_SUFFIX = (SELECT MAX(suffix) FROM u)";
        let diagnostics = analyse_sql(sql);

        assert!(
            diagnostics
                .iter()
                .any(|d| d.to_string().contains("CURRENT_DATE")),
            "expected a CURRENT_DATE diagnostic, got: {:?}",
            diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.to_string().contains("Full scan")),
            "expected a full-scan diagnostic, got: {:?}",
            diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn analyse_sql_does_not_panic_on_garbage_input() {
        // Unparseable input must degrade gracefully (no panic), not crash the tool.
        let _ = analyse_sql("!@#$ not really ;; SQL (((");
        let _ = analyse_sql("SELECT FROM WHERE");
    }
}
