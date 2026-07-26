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
        let d = Diagnostic::new(3, 5, "boom".to_string());
        assert_eq!(d.row(), 3);
        assert_eq!(d.col(), 5);
        assert_eq!(d.message(), "boom");
    }

    #[test]
    fn display_formats_as_row_col_message() {
        let d = Diagnostic::new(3, 5, "boom".to_string());
        assert_eq!(format!("{}", d), "3:5: boom");
    }
}
