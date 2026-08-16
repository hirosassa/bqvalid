use bqvalid::ast::Ast;
use bqvalid::config::{self, Config};
use bqvalid::diagnostic::Diagnostic;
use bqvalid::output::{self, FileResult, OutputFormat};
use bqvalid::rules::{known_rule_ids, run_rules_ignoring};
use clap::Parser;
use clap_verbosity_flag::Verbosity;
use googlesql::Module;
use log::debug;
use rayon::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Stdin};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
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

    /// Output format for diagnostics.
    #[clap(long, value_enum, default_value_t = OutputFormat::Plain)]
    format: OutputFormat,

    /// Rule id to ignore (suppress its diagnostics). Accepts a comma-separated
    /// list and is repeatable. When given, overrides the `ignore` list from the
    /// config file.
    #[clap(long, value_name = "RULE_ID", value_delimiter = ',')]
    ignore: Vec<String>,

    /// Path to a TOML config file. Defaults to `bqvalid.toml` in the current
    /// directory when present.
    #[clap(long, value_name = "PATH")]
    config: Option<PathBuf>,

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

    let ignore = match resolve_ignore(args.config, args.ignore) {
        Ok(ignore) => ignore,
        Err(e) => {
            eprintln!("Error: {}", e);
            return ExitCode::FAILURE;
        }
    };

    // Both inputs converge on `Vec<FileResult>`; `show_paths` only distinguishes
    // the two in the plain format (files prefix the path, stdin does not).
    let (results, show_paths) = if args.files.is_empty() {
        match analyse_stdin(&stdin, &ignore) {
            Some(results) => (results, false),
            None => return ExitCode::FAILURE,
        }
    } else {
        (analyse_paths(collect_targets(args.files), &ignore), true)
    };

    let mut out = io::stdout().lock();
    let mut err = io::stderr().lock();
    match output::emit(
        &results,
        args.format,
        get_version(),
        show_paths,
        &mut out,
        &mut err,
    ) {
        Ok(true) => ExitCode::FAILURE,
        Ok(false) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error writing output: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// Read SQL from stdin and analyse it as a single `<stdin>` result. Returns
/// `None` (after logging to stderr) when the input cannot be read or the parser
/// module fails to load, so the caller can exit with a failure code.
fn analyse_stdin(stdin: &Stdin, ignore: &HashSet<String>) -> Option<Vec<FileResult>> {
    let mut sql = String::new();
    let read_result = stdin.lock().read_to_string(&mut sql);
    if let Err(e) = read_result {
        eprintln!("Error reading stdin: {}", e);
        return None;
    }
    let mut module = new_module()?;
    Some(vec![FileResult {
        path: PathBuf::from("<stdin>"),
        diagnostics: analyse_sql_googlesql(&mut module, &sql, ignore),
        read_error: None,
    }])
}

/// Expand the CLI file arguments into the set of `.sql` files to analyse,
/// walking directories recursively. Walk errors are logged to stderr and
/// skipped.
fn collect_targets(files: Vec<String>) -> Vec<PathBuf> {
    files
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
        .collect()
}

fn is_sql(entry: &DirEntry) -> bool {
    entry
        .path()
        .extension()
        .map(|s| s == "sql")
        .unwrap_or(false)
}

/// Build a googlesql (ZetaSQL) parser module. Returns `None` if the module fails
/// to initialize (logged to stderr).
///
/// The parser uses googlesql's native-ffi backend; see [`bqvalid::build_module`].
fn new_module() -> Option<Module> {
    match bqvalid::build_module() {
        Ok(module) => Some(module),
        Err(e) => {
            eprintln!("Error initializing googlesql parser: {}", e);
            None
        }
    }
}

/// Analyse many files in parallel. Each worker thread builds its parser module
/// once (via `map_init`) and reuses it across the files it handles, so the
/// module is initialized per thread rather than per file. A `None` module means
/// initialization failed and was already reported; that thread's files then
/// yield no diagnostics. Results are sorted by path so the output is stable
/// regardless of scheduling.
///
/// The `Module` is large, so it is boxed to keep the per-item closure state
/// small.
fn analyse_paths(paths: Vec<PathBuf>, ignore: &HashSet<String>) -> Vec<FileResult> {
    let mut results: Vec<FileResult> = paths
        .par_iter()
        .map_init(
            || new_module().map(Box::new),
            |module, path| match fs::read_to_string(path) {
                Ok(sql) => {
                    let diagnostics = module.as_mut().map_or_else(Vec::new, |module| {
                        analyse_sql_googlesql(module.as_mut(), &sql, ignore)
                    });
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
            },
        )
        .collect();
    results.sort_by(|a, b| a.path.cmp(&b.path));
    results
}

/// Analyse `sql` with the googlesql (ZetaSQL) backend.
///
/// ZetaSQL parses one statement at a time and cannot recover past a syntax
/// error, so `parse_statements` returns every statement it parsed before
/// stopping plus the error that stopped it. We run the rules over each parsed
/// statement (byte offsets are relative to the whole script, so positions stay
/// correct) and report the halting error, if any, to stderr, still emitting
/// diagnostics for the statements it could parse.
fn analyse_sql_googlesql(
    module: &mut Module,
    sql: &str,
    ignore: &HashSet<String>,
) -> Vec<Diagnostic> {
    let parsed = match module.parse_statements(sql) {
        Ok(parsed) => parsed,
        Err(e) => {
            eprintln!("Error parsing SQL input: {}", e);
            return Vec::new();
        }
    };
    if let Some(error) = parsed.error() {
        eprintln!("Error parsing SQL input: {}", error);
    }

    let mut diagnostics = Vec::new();
    for statement in parsed.statements() {
        let ast = Ast::from_googlesql_root(statement.root(), sql);
        diagnostics.extend(run_rules_ignoring(&ast, sql, ignore));
    }
    diagnostics
}

/// Resolve the effective set of ignored rule ids from the config file and CLI.
/// Discovers the config by walking up from the current directory to the git
/// repository root, lets a non-empty CLI `--ignore` override it, and warns
/// about ids that match no known rule. Returns an error string when the config
/// cannot be loaded.
fn resolve_ignore(
    config_path: Option<PathBuf>,
    cli_ignore: Vec<String>,
) -> Result<HashSet<String>, String> {
    let cwd = std::env::current_dir()
        .map_err(|e| format!("cannot determine current directory: {}", e))?;
    resolve_ignore_in(&cwd, config_path, cli_ignore)
}

/// Core of [`resolve_ignore`], parameterized by the directory used to discover
/// the default config file so it can be exercised without touching the process
/// working directory.
fn resolve_ignore_in(
    cwd: &Path,
    config_path: Option<PathBuf>,
    cli_ignore: Vec<String>,
) -> Result<HashSet<String>, String> {
    let config = match config::discover_config(config_path, cwd) {
        Some(path) => Config::load(&path).map_err(|e| e.to_string())?,
        None => Config::default(),
    };
    let ignore = config::effective_ignore(cli_ignore, config.ignore);
    for id in config::unknown_ignore_ids(&ignore, &known_rule_ids()) {
        eprintln!("Warning: unknown rule id in ignore list: {}", id);
    }
    Ok(ignore.into_iter().collect())
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

    /// Build a googlesql module and run the full rule set over `sql`, mirroring
    /// how the binary drives `analyse_sql_googlesql`.
    fn analyse(sql: &str) -> Vec<Diagnostic> {
        let mut module = new_module().expect("googlesql module builds");
        analyse_sql_googlesql(&mut module, sql, &HashSet::new())
    }

    #[test]
    fn flags_current_date_end_to_end() {
        // The whole pipeline (ZetaSQL parse -> neutral AST -> rules) must surface
        // the use_current_date rule with a 1-based position.
        let diagnostics = analyse("SELECT CURRENT_DATE() FROM t");
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].row(), 1);
        assert_eq!(diagnostics[0].col(), 8);
    }

    #[test]
    fn analyses_every_statement_in_a_script() {
        // parse_statements recovers at statement boundaries, so a CURRENT_DATE in
        // the second statement is still flagged, with a position relative to the
        // whole script (row 2).
        let sql = "SELECT id FROM a;\nSELECT CURRENT_DATE() FROM b";
        let diagnostics = analyse(sql);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].row(), 2);
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
        // A single statement that trips more than one rule (use_current_date and
        // compare_table_suffix_with_subquery) yields multiple diagnostics.
        let sql = "\
select
  current_date,
  column_a
from
  dataset.table
where
  _table_suffix between '2022-06-01'
  and (
    select dt from dates
  )
";
        let diagnostics = analyse(sql);
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
    fn analyse_sql_reuses_a_single_module_across_calls() {
        // One module instance handles many inputs without reinitializing. Each
        // parse must stay independent (no state leaking between calls), so a clean
        // query after a dirty one still yields nothing.
        let mut module = new_module().expect("googlesql module builds");
        let dirty = analyse_sql_googlesql(&mut module, "SELECT CURRENT_DATE()", &HashSet::new());
        let clean = analyse_sql_googlesql(&mut module, "SELECT id FROM users", &HashSet::new());
        assert!(!dirty.is_empty(), "dirty query should produce diagnostics");
        assert!(clean.is_empty(), "clean query should produce none");
    }

    #[test]
    fn analyse_sql_aggregates_multiple_rules_from_a_single_query() {
        // Triggers both use_current_date and compare_table_suffix_with_subquery,
        // proving analysis fans a query out across every rule and merges results.
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
        // Unparseable input must degrade gracefully (the parse error is reported
        // to stderr and yields no diagnostics), not crash the tool.
        assert!(analyse("!@#$ not really ;; SQL (((").is_empty());
        assert!(analyse("SELECT FROM WHERE").is_empty());
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

        let results = analyse_paths(paths, &HashSet::new());

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

        let results = analyse_paths(vec![missing.clone()], &HashSet::new());

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, missing);
        assert!(results[0].read_error.is_some());
        assert!(results[0].diagnostics.is_empty());
    }

    #[test]
    fn resolve_ignore_reads_the_config_ignore_list() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("bqvalid.toml"),
            "ignore = [\"use_current_date\"]",
        )
        .unwrap();

        let ignore = resolve_ignore_in(dir.path(), None, Vec::new()).expect("loads config");
        assert_eq!(
            ignore,
            std::iter::once("use_current_date".to_string()).collect()
        );
    }

    #[test]
    fn resolve_ignore_cli_overrides_config_file() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("bqvalid.toml"),
            "ignore = [\"use_current_date\"]",
        )
        .unwrap();

        let ignore = resolve_ignore_in(dir.path(), None, vec!["invalid_group_by".to_string()])
            .expect("loads config");
        assert_eq!(
            ignore,
            std::iter::once("invalid_group_by".to_string()).collect(),
            "CLI --ignore replaces the config list"
        );
    }

    #[test]
    fn resolve_ignore_is_empty_without_config_or_cli() {
        let dir = tempdir().unwrap();
        let ignore = resolve_ignore_in(dir.path(), None, Vec::new()).expect("no config is fine");
        assert!(ignore.is_empty());
    }

    #[test]
    fn ignore_flag_accepts_comma_separated_rules() {
        let args = Args::try_parse_from([
            "bqvalid",
            "--ignore",
            "use_current_date,unnecessary_order_by",
            "x.sql",
        ])
        .expect("parses");
        assert_eq!(
            args.ignore,
            vec![
                "use_current_date".to_string(),
                "unnecessary_order_by".to_string()
            ]
        );
    }

    #[test]
    fn ignore_flag_still_accepts_repeated_flags() {
        let args = Args::try_parse_from(["bqvalid", "--ignore", "a", "--ignore", "b", "x.sql"])
            .expect("parses");
        assert_eq!(args.ignore, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn resolve_ignore_reports_a_broken_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("custom.toml");
        fs::write(&path, "ignore = not-a-list").unwrap();

        let err = resolve_ignore_in(dir.path(), Some(path), Vec::new());
        assert!(err.is_err(), "a malformed config must be a hard error");
    }

    #[test]
    fn resolve_ignore_reports_a_missing_explicit_config() {
        // An explicit --config that cannot be read must fail loudly rather than
        // being silently treated as "no config".
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does_not_exist.toml");

        let err = resolve_ignore_in(dir.path(), Some(missing), Vec::new());
        assert!(err.is_err(), "a missing explicit config must be an error");
    }

    #[test]
    fn resolve_ignore_keeps_unknown_ids_without_failing() {
        // An unknown rule id is warned about on stderr but must not be dropped
        // or turned into an error: downstream it simply matches no rule.
        let dir = tempdir().unwrap();

        let ignore = resolve_ignore_in(
            dir.path(),
            None,
            vec!["use_current_date".to_string(), "not_a_rule".to_string()],
        )
        .expect("unknown ids are not an error");

        let expected: HashSet<String> = ["use_current_date".to_string(), "not_a_rule".to_string()]
            .into_iter()
            .collect();
        assert_eq!(ignore, expected, "unknown ids are retained, not dropped");
    }
}
