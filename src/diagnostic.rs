use std::fmt::Display;

/// Severity of a diagnostic.
///
/// `Error` marks a query that BigQuery would reject at runtime; `Warning` marks
/// a performance or maintainability problem that still runs. The human-readable
/// output does not print the severity yet — it is stored so machine-readable
/// formats and per-rule control can use it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// Represents a diagnostic, such as a full scan error.
///
/// rows and columns are 1-based.
pub struct Diagnostic {
    rule_id: &'static str,
    severity: Severity,
    row: usize,
    col: usize,
    message: String,
}

impl Diagnostic {
    pub const fn new(
        rule_id: &'static str,
        severity: Severity,
        row: usize,
        col: usize,
        message: String,
    ) -> Self {
        Self {
            rule_id,
            severity,
            row,
            col,
            message,
        }
    }

    pub const fn rule_id(&self) -> &'static str {
        self.rule_id
    }

    pub const fn severity(&self) -> Severity {
        self.severity
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub const fn row(&self) -> usize {
        self.row
    }

    pub const fn col(&self) -> usize {
        self.col
    }
}

impl Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.row, self.col, self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accessors_return_the_constructed_values() {
        let d = Diagnostic::new("some_rule", Severity::Warning, 3, 5, "boom".to_string());
        assert_eq!(d.rule_id(), "some_rule");
        assert_eq!(d.severity(), Severity::Warning);
        assert_eq!(d.row(), 3);
        assert_eq!(d.col(), 5);
        assert_eq!(d.message(), "boom");
    }

    #[test]
    fn severity_is_preserved() {
        let d = Diagnostic::new("some_rule", Severity::Error, 1, 1, "bad".to_string());
        assert_eq!(d.severity(), Severity::Error);
    }

    #[test]
    fn display_formats_as_row_col_message() {
        // The human-readable form stays row:col: message; severity/rule_id are not
        // printed here so existing output and pipelines are unchanged.
        let d = Diagnostic::new("some_rule", Severity::Warning, 3, 5, "boom".to_string());
        assert_eq!(format!("{}", d), "3:5: boom");
    }
}
