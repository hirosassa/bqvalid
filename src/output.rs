//! Rendering of diagnostics in every output format.
//!
//! `emit()` is the single entry point for both the stdin and files paths: it
//! writes lint diagnostics to `out` in the selected format and the tool's own
//! read failures to `err`. The `plain` format keeps the stdin/files
//! distinction via the `show_paths` flag (files prefix each line with the
//! path; stdin does not). The `json`/`sarif` formats build an aggregated
//! document for the whole run.

use crate::diagnostic::{Diagnostic, Severity};
use serde_json::{Value, json};
use std::io::{self, Write};
use std::path::PathBuf;

/// Output format selected via `--format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable `row:col: message` (optionally path-prefixed). Default.
    Plain,
    /// A single JSON document with a flat `diagnostics` array.
    Json,
    /// SARIF 2.1.0, for GitHub code scanning and editor integrations.
    Sarif,
}

/// One file's diagnostics, as seen by the machine-readable formatters. For the
/// stdin path the `path` is a placeholder such as `<stdin>`.
pub struct FileDiagnostics<'a> {
    pub path: &'a str,
    pub diagnostics: &'a [Diagnostic],
}

/// Result of analysing a single input. For the stdin path the `path` is a
/// placeholder such as `<stdin>`. `read_error` is set when the file could not
/// be read; in that case `diagnostics` is empty.
pub struct FileResult {
    pub path: PathBuf,
    pub diagnostics: Vec<Diagnostic>,
    pub read_error: Option<String>,
}

/// Render `results` in the selected `format`.
///
/// Lint diagnostics go to `out` (stdout, pipeable), while the tool's own read
/// failures go to `err` (stderr), so the two never mix on the same pipe.
/// Returns `true` when any diagnostic or read error was seen, so the caller can
/// pick the exit code.
///
/// `show_paths` only affects the `plain` format: the files path prefixes each
/// line with the file path, while the stdin path (a single result) does not.
/// The machine formats always carry the path per diagnostic.
pub fn emit<O: Write, E: Write>(
    results: &[FileResult],
    format: OutputFormat,
    version: &str,
    show_paths: bool,
    out: &mut O,
    err: &mut E,
) -> io::Result<bool> {
    let mut has_problem = false;
    for result in results {
        if let Some(read_error) = &result.read_error {
            writeln!(
                err,
                "{}: Error reading file: {}",
                result.path.display(),
                read_error
            )?;
            has_problem = true;
        }
        if !result.diagnostics.is_empty() {
            has_problem = true;
        }
    }

    match format {
        OutputFormat::Plain => write_plain(out, results, show_paths)?,
        OutputFormat::Json | OutputFormat::Sarif => {
            // `path.display()` yields a temporary, so materialize the path
            // strings first and borrow them into the format views.
            let paths: Vec<String> = results
                .iter()
                .map(|r| r.path.display().to_string())
                .collect();
            let views: Vec<FileDiagnostics> = results
                .iter()
                .zip(&paths)
                .map(|(r, path)| FileDiagnostics {
                    path,
                    diagnostics: &r.diagnostics,
                })
                .collect();
            match format {
                OutputFormat::Json => write_json(out, &views)?,
                OutputFormat::Sarif => write_sarif(out, &views, version)?,
                OutputFormat::Plain => {}
            }
        }
    }

    Ok(has_problem)
}

/// Write lint diagnostics in the human-readable `plain` format. When
/// `show_paths` is set each line is prefixed with the file path.
fn write_plain<W: Write>(out: &mut W, results: &[FileResult], show_paths: bool) -> io::Result<()> {
    for result in results {
        for diagnostic in &result.diagnostics {
            if show_paths {
                writeln!(out, "{}: {}", result.path.display(), diagnostic)?;
            } else {
                writeln!(out, "{}", diagnostic)?;
            }
        }
    }
    Ok(())
}

/// Lowercase severity string shared by JSON (`severity`) and SARIF (`level`).
/// SARIF levels are drawn from the same vocabulary (`warning`, `error`).
const fn severity_str(severity: Severity) -> &'static str {
    match severity {
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

/// Serialize `value` as pretty JSON followed by a trailing newline.
fn write_json_value<W: Write>(out: &mut W, value: &Value) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, value).map_err(io::Error::other)?;
    writeln!(out)
}

/// Write all diagnostics as a single JSON document with a flat `diagnostics` array.
///
/// Each entry carries its file path, rule id, severity and 1-based coordinates
/// so CI tooling can consume them without parsing the plain text.
pub fn write_json<W: Write>(out: &mut W, files: &[FileDiagnostics]) -> io::Result<()> {
    let diagnostics: Vec<Value> = files
        .iter()
        .flat_map(|file| {
            file.diagnostics.iter().map(move |d| {
                json!({
                    "path": file.path,
                    "rule_id": d.rule_id(),
                    "severity": severity_str(d.severity()),
                    "row": d.row(),
                    "col": d.col(),
                    "message": d.message(),
                })
            })
        })
        .collect();

    write_json_value(out, &json!({ "diagnostics": diagnostics }))
}

/// Write all diagnostics as a SARIF 2.1.0 document. `version` is the tool
/// version reported in `tool.driver.version`.
pub fn write_sarif<W: Write>(
    out: &mut W,
    files: &[FileDiagnostics],
    version: &str,
) -> io::Result<()> {
    let results: Vec<Value> = files
        .iter()
        .flat_map(|file| {
            file.diagnostics.iter().map(move |d| {
                json!({
                    "ruleId": d.rule_id(),
                    "level": severity_str(d.severity()),
                    "message": { "text": d.message() },
                    "locations": [{
                        "physicalLocation": {
                            "artifactLocation": { "uri": file.path },
                            "region": { "startLine": d.row(), "startColumn": d.col() },
                        }
                    }],
                })
            })
        })
        .collect();

    // Advertise the distinct rules that produced results, in a stable order.
    let mut rule_ids: Vec<&str> = files
        .iter()
        .flat_map(|file| file.diagnostics.iter().map(Diagnostic::rule_id))
        .collect();
    rule_ids.sort_unstable();
    rule_ids.dedup();
    let rules: Vec<Value> = rule_ids.iter().map(|id| json!({ "id": id })).collect();

    let doc = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "bqvalid",
                    "informationUri": "https://github.com/hirosassa/bqvalid",
                    "version": version,
                    "rules": rules,
                }
            },
            "results": results,
        }],
    });

    write_json_value(out, &doc)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]
mod tests {
    use super::*;

    fn sample() -> Vec<Diagnostic> {
        vec![
            Diagnostic::new(
                "use_current_date",
                Severity::Warning,
                2,
                3,
                "Don't use CURRENT_DATE".to_string(),
            ),
            Diagnostic::new(
                "invalid_group_by",
                Severity::Error,
                5,
                1,
                "Not in GROUP BY".to_string(),
            ),
        ]
    }

    fn render_json(files: &[FileDiagnostics]) -> Value {
        let mut buf = Vec::new();
        write_json(&mut buf, files).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    fn render_sarif(files: &[FileDiagnostics], version: &str) -> Value {
        let mut buf = Vec::new();
        write_sarif(&mut buf, files, version).unwrap();
        serde_json::from_slice(&buf).unwrap()
    }

    #[test]
    fn json_emits_one_entry_per_diagnostic_with_all_fields() {
        let diags = sample();
        let files = vec![FileDiagnostics {
            path: "a.sql",
            diagnostics: &diags,
        }];
        let doc = render_json(&files);

        let entries = doc["diagnostics"].as_array().unwrap();
        assert_eq!(entries.len(), 2);

        let first = &entries[0];
        assert_eq!(first["path"], "a.sql");
        assert_eq!(first["rule_id"], "use_current_date");
        assert_eq!(first["severity"], "warning");
        assert_eq!(first["row"], 2);
        assert_eq!(first["col"], 3);
        assert_eq!(first["message"], "Don't use CURRENT_DATE");

        // Error severity is rendered as "error".
        assert_eq!(entries[1]["severity"], "error");
    }

    #[test]
    fn json_aggregates_diagnostics_across_files() {
        let a = sample();
        let b = sample();
        let files = vec![
            FileDiagnostics {
                path: "a.sql",
                diagnostics: &a,
            },
            FileDiagnostics {
                path: "b.sql",
                diagnostics: &b,
            },
        ];
        let doc = render_json(&files);
        assert_eq!(doc["diagnostics"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn json_with_no_diagnostics_is_an_empty_array() {
        let files: Vec<FileDiagnostics> = Vec::new();
        let doc = render_json(&files);
        assert!(doc["diagnostics"].as_array().unwrap().is_empty());
    }

    #[test]
    fn sarif_has_the_2_1_0_envelope_and_tool_driver() {
        let diags = sample();
        let files = vec![FileDiagnostics {
            path: "a.sql",
            diagnostics: &diags,
        }];
        let doc = render_sarif(&files, "1.2.3");

        assert_eq!(doc["version"], "2.1.0");
        let driver = &doc["runs"][0]["tool"]["driver"];
        assert_eq!(driver["name"], "bqvalid");
        assert_eq!(driver["version"], "1.2.3");
    }

    #[test]
    fn sarif_maps_each_diagnostic_to_a_result_with_location_and_level() {
        let diags = sample();
        let files = vec![FileDiagnostics {
            path: "a.sql",
            diagnostics: &diags,
        }];
        let doc = render_sarif(&files, "1.2.3");

        let results = doc["runs"][0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);

        let first = &results[0];
        assert_eq!(first["ruleId"], "use_current_date");
        assert_eq!(first["level"], "warning");
        assert_eq!(first["message"]["text"], "Don't use CURRENT_DATE");

        let region = &first["locations"][0]["physicalLocation"]["region"];
        assert_eq!(region["startLine"], 2);
        assert_eq!(region["startColumn"], 3);
        let uri = &first["locations"][0]["physicalLocation"]["artifactLocation"]["uri"];
        assert_eq!(uri, "a.sql");

        // Error severity maps to the SARIF "error" level.
        assert_eq!(results[1]["level"], "error");
    }

    #[test]
    fn sarif_lists_distinct_rules_sorted_and_deduped() {
        // Two files each trip both rules; the driver should list each rule once.
        let a = sample();
        let b = sample();
        let files = vec![
            FileDiagnostics {
                path: "a.sql",
                diagnostics: &a,
            },
            FileDiagnostics {
                path: "b.sql",
                diagnostics: &b,
            },
        ];
        let doc = render_sarif(&files, "1.2.3");

        let rules = doc["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap();
        let ids: Vec<&str> = rules.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["invalid_group_by", "use_current_date"]);
    }

    #[test]
    fn emit_plain_without_paths_writes_bare_diagnostics() {
        // The stdin path (show_paths = false) emits `row:col: message` with no
        // path prefix.
        let results = vec![FileResult {
            path: PathBuf::from("<stdin>"),
            diagnostics: vec![
                Diagnostic::new("test_rule", Severity::Warning, 1, 1, "first".to_string()),
                Diagnostic::new("test_rule", Severity::Warning, 2, 3, "second".to_string()),
            ],
            read_error: None,
        }];

        let mut out = Vec::new();
        let mut err = Vec::new();
        let has_problem = emit(
            &results,
            OutputFormat::Plain,
            "1.2.3",
            false,
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(has_problem);
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("first"));
        assert!(out.contains("second"));
        assert!(
            !out.contains("<stdin>"),
            "stdin diagnostics must not be path-prefixed"
        );
        assert!(err.is_empty());
    }

    #[test]
    fn emit_plain_with_paths_prefixes_stdout_and_read_errors_go_to_stderr() {
        // The files path (show_paths = true): diagnostics (normal lint output)
        // go to out with a path prefix; the tool's own failures (unreadable
        // file) go to err, so the two never mix on the same pipe.
        let results = vec![
            FileResult {
                path: PathBuf::from("good.sql"),
                diagnostics: vec![Diagnostic::new(
                    "test_rule",
                    Severity::Warning,
                    1,
                    8,
                    "some warning".to_string(),
                )],
                read_error: None,
            },
            FileResult {
                path: PathBuf::from("missing.sql"),
                diagnostics: Vec::new(),
                read_error: Some("no such file".to_string()),
            },
        ];

        let mut out = Vec::new();
        let mut err = Vec::new();
        let has_problem = emit(
            &results,
            OutputFormat::Plain,
            "1.2.3",
            true,
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(
            has_problem,
            "a diagnostic or read error must flag a problem"
        );
        let out = String::from_utf8(out).unwrap();
        let err = String::from_utf8(err).unwrap();
        assert!(
            out.contains("good.sql: "),
            "diagnostic must be path-prefixed on stdout"
        );
        assert!(out.contains("some warning"));
        assert!(
            !out.contains("Error reading file"),
            "tool errors must not appear on stdout"
        );
        assert!(
            err.contains("Error reading file"),
            "read error must land on stderr"
        );
        assert!(
            !err.contains("some warning"),
            "diagnostics must not appear on stderr"
        );
    }

    #[test]
    fn emit_flags_no_problem_for_clean_results() {
        let results = vec![FileResult {
            path: PathBuf::from("clean.sql"),
            diagnostics: Vec::new(),
            read_error: None,
        }];

        let mut out = Vec::new();
        let mut err = Vec::new();
        let has_problem = emit(
            &results,
            OutputFormat::Plain,
            "1.2.3",
            true,
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(!has_problem);
        assert!(out.is_empty());
        assert!(err.is_empty());
    }

    #[test]
    fn emit_json_writes_document_to_stdout_and_read_errors_to_stderr() {
        // A machine format aggregates diagnostics into a single JSON document on
        // stdout; the tool's own read failure still goes to stderr so the two
        // never mix on the same pipe.
        let results = vec![
            FileResult {
                path: PathBuf::from("good.sql"),
                diagnostics: vec![Diagnostic::new(
                    "use_current_date",
                    Severity::Warning,
                    1,
                    8,
                    "some warning".to_string(),
                )],
                read_error: None,
            },
            FileResult {
                path: PathBuf::from("missing.sql"),
                diagnostics: Vec::new(),
                read_error: Some("no such file".to_string()),
            },
        ];

        let mut out = Vec::new();
        let mut err = Vec::new();
        let has_problem = emit(
            &results,
            OutputFormat::Json,
            "1.2.3",
            true,
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(has_problem);
        let doc: Value = serde_json::from_slice(&out).unwrap();
        let entries = doc["diagnostics"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["rule_id"], "use_current_date");
        assert_eq!(entries[0]["path"], "good.sql");

        let err = String::from_utf8(err).unwrap();
        assert!(err.contains("Error reading file"));
        assert!(
            !err.contains("some warning"),
            "diagnostics must not appear on stderr"
        );
    }

    #[test]
    fn emit_sarif_produces_a_2_1_0_run() {
        let results = vec![FileResult {
            path: PathBuf::from("a.sql"),
            diagnostics: vec![Diagnostic::new(
                "invalid_group_by",
                Severity::Error,
                5,
                1,
                "bad".to_string(),
            )],
            read_error: None,
        }];

        let mut out = Vec::new();
        let mut err = Vec::new();
        let has_problem = emit(
            &results,
            OutputFormat::Sarif,
            "1.2.3",
            true,
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(has_problem);
        let doc: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(doc["version"], "2.1.0");
        let result = &doc["runs"][0]["results"][0];
        assert_eq!(result["ruleId"], "invalid_group_by");
        assert_eq!(result["level"], "error");
    }

    #[test]
    fn emit_json_flags_no_problem_for_clean_results() {
        let results = vec![FileResult {
            path: PathBuf::from("clean.sql"),
            diagnostics: Vec::new(),
            read_error: None,
        }];

        let mut out = Vec::new();
        let mut err = Vec::new();
        let has_problem = emit(
            &results,
            OutputFormat::Json,
            "1.2.3",
            true,
            &mut out,
            &mut err,
        )
        .unwrap();

        assert!(!has_problem);
        // Still a well-formed document, just with no diagnostics.
        let doc: Value = serde_json::from_slice(&out).unwrap();
        assert!(doc["diagnostics"].as_array().unwrap().is_empty());
        assert!(err.is_empty());
    }
}
