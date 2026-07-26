//! Machine-readable rendering of diagnostics.
//!
//! The human-readable `plain` format stays in `main.rs` (it keeps the
//! stdin/files path distinction). This module builds the aggregated `json` and
//! `sarif` documents, which need a path per diagnostic and emit a single
//! document for the whole run.

use crate::diagnostic::{Diagnostic, Severity};
use serde_json::{Value, json};
use std::io::{self, Write};

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
}
