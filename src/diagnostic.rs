use std::fmt::Display;

/// Represents a diagnostic, such as a full scan error.
///
/// rows and columns are 1-based.
pub struct Diagnostic {
    row: usize,
    col: usize,
    message: String,
}

impl Diagnostic {
    pub const fn new(row: usize, col: usize, message: String) -> Self {
        Self { row, col, message }
    }
}

impl Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.row, self.col, self.message)
    }
}
