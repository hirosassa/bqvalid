use bqvalid::diagnostic::Diagnostic;
use bqvalid::rules::compare_table_suffix_with_subquery;
use bqvalid::rules::invalid_group_by;
use bqvalid::rules::unnecessary_order_by;
use bqvalid::rules::unused_column_in_cte;
use bqvalid::rules::use_current_date;
use clap::Parser;
use clap_verbosity_flag::Verbosity;
use log::debug;
use rayon::prelude::*;
use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;
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
        let Some(mut parser) = new_parser() else {
            return ExitCode::FAILURE;
        };
        let diagnostics = analyse_sql(&mut parser, &sql);
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
    let targets: Vec<PathBuf> = args
        .files
        .into_iter()
        .flat_map(|f| {
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
                .map(DirEntry::into_path)
        })
        .collect();

    let results = analyse_paths(targets);

    let mut has_error = false;
    let mut has_diagnostics = false;
    for result in &results {
        if let Some(err) = &result.read_error {
            eprintln!("{}: Error reading file: {}", result.path.display(), err);
            has_error = true;
        }
        for diagnostic in &result.diagnostics {
            eprintln!("{}: {}", result.path.display(), diagnostic);
            has_diagnostics = true;
        }
    }

    if has_error || has_diagnostics {
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

/// Result of analysing a single file. `read_error` is set when the file could
/// not be read; in that case `diagnostics` is empty.
struct FileResult {
    path: PathBuf,
    diagnostics: Vec<Diagnostic>,
    read_error: Option<String>,
}

/// Build a parser with the BigQuery grammar loaded once. Returns `None` if the
/// grammar fails to load (logged to stderr).
fn new_parser() -> Option<TsParser> {
    let mut parser = TsParser::new();
    match parser.set_language(&language()) {
        Ok(()) => Some(parser),
        Err(e) => {
            eprintln!("Error loading BigQuery grammar: {}", e);
            None
        }
    }
}

/// Analyse many files in parallel. Each worker thread builds its parser once
/// (via `map_init`) and reuses it across the files it handles, so the grammar
/// is loaded per thread rather than per file. Results are sorted by path so the
/// output is stable regardless of scheduling.
fn analyse_paths(paths: Vec<PathBuf>) -> Vec<FileResult> {
    let mut results: Vec<FileResult> = paths
        .par_iter()
        .map_init(new_parser, |parser, path| match fs::read_to_string(path) {
            Ok(sql) => {
                let diagnostics = parser
                    .as_mut()
                    .map_or_else(Vec::new, |parser| analyse_sql(parser, &sql));
                FileResult {
                    path: path.clone(),
                    diagnostics,
                    read_error: None,
                }
            }
            Err(e) => FileResult {
                path: path.clone(),
                diagnostics: Vec::new(),
                read_error: Some(e.to_string()),
            },
        })
        .collect();
    results.sort_by(|a, b| a.path.cmp(&b.path));
    results
}

fn analyse_sql(parser: &mut TsParser, sql: &str) -> Vec<Diagnostic> {
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

    /// Build a parser and run the full rule set over `sql`, mirroring how the
    /// binary drives `analyse_sql`.
    fn analyse(sql: &str) -> Vec<Diagnostic> {
        let mut parser = new_parser().expect("grammar loads");
        analyse_sql(&mut parser, sql)
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
        assert!(analyse("").is_empty());
    }

    #[test]
    fn analyse_sql_clean_query_yields_no_diagnostics() {
        // A plain, well-formed query with none of the linted anti-patterns.
        assert!(analyse("SELECT id, name FROM users").is_empty());
    }

    #[test]
    fn analyse_sql_reuses_a_single_parser_across_calls() {
        // P1: one parser instance handles many inputs without reloading the
        // grammar. Each parse must stay independent (no state leaking between
        // calls), so a clean query after a dirty one still yields nothing.
        let mut parser = new_parser().expect("grammar loads");
        let dirty = analyse_sql(&mut parser, "SELECT CURRENT_DATE()");
        let clean = analyse_sql(&mut parser, "SELECT id FROM users");
        assert!(!dirty.is_empty(), "dirty query should produce diagnostics");
        assert!(clean.is_empty(), "clean query should produce none");
    }

    #[test]
    fn analyse_sql_aggregates_multiple_rules_from_a_single_query() {
        // Triggers both use_current_date and compare_table_suffix_with_subquery,
        // proving analyse_sql fans a query out across every rule and merges results.
        let sql = "SELECT CURRENT_DATE() AS d \
                   FROM t \
                   WHERE _TABLE_SUFFIX = (SELECT MAX(suffix) FROM u)";
        let diagnostics = analyse(sql);

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
        let _ = analyse("!@#$ not really ;; SQL (((");
        let _ = analyse("SELECT FROM WHERE");
    }

    #[test]
    fn analyse_paths_sorts_results_by_path_and_aggregates_all_files() {
        // P3 runs files in parallel, so scheduling order is nondeterministic.
        // analyse_paths must still return one result per file, ordered by path,
        // and carry every file's diagnostics.
        let dir = tempdir().unwrap();
        // Create in non-alphabetical order; each file trips CURRENT_DATE.
        for name in ["c.sql", "a.sql", "b.sql"] {
            fs::write(dir.path().join(name), "SELECT CURRENT_DATE()").unwrap();
        }
        let paths = vec![
            dir.path().join("c.sql"),
            dir.path().join("a.sql"),
            dir.path().join("b.sql"),
        ];

        let results = analyse_paths(paths);

        let ordered: Vec<PathBuf> = results.iter().map(|r| r.path.clone()).collect();
        let mut expected = ordered.clone();
        expected.sort();
        assert_eq!(ordered, expected, "results must be sorted by path");

        assert_eq!(results.len(), 3, "every file must be represented");
        assert!(
            results.iter().all(|r| !r.diagnostics.is_empty()),
            "each file's diagnostics must be aggregated"
        );
    }

    #[test]
    fn analyse_paths_records_read_errors_for_missing_files() {
        // A path that cannot be read surfaces as a read_error, not a silent drop.
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does_not_exist.sql");

        let results = analyse_paths(vec![missing.clone()]);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, missing);
        assert!(results[0].read_error.is_some());
        assert!(results[0].diagnostics.is_empty());
    }
}
