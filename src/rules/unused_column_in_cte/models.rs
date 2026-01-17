use std::{cmp::Ord, fmt::Display};

/// Represents a column in a SQL query
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub table_name: Option<String>,
    pub column_name: String,
    /// Original column name before alias (if aliased)
    /// e.g., "column1" for "column1 AS unique_id"
    pub original_column_name: Option<String>,
    pub row: usize,
    pub col: usize,
}

impl ColumnInfo {
    // Note: While this function could technically be declared as `const fn`,
    // it cannot be called in const contexts because it takes `String` parameters
    // which require non-const operations like `to_string()` to construct.
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(
        table_name: Option<String>,
        column_name: String,
        original_column_name: Option<String>,
        row: usize,
        col: usize,
    ) -> Self {
        Self {
            table_name,
            column_name,
            original_column_name,
            row: row + 1,
            col: col + 1,
        }
    }
}

impl Display for ColumnInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let table_name = self.table_name.clone().unwrap_or_default();
        write!(
            f,
            "{}:{}:{}:{}",
            table_name, self.column_name, self.row, self.col
        )
    }
}

impl PartialOrd for ColumnInfo {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ColumnInfo {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.row.cmp(&other.row).then(self.col.cmp(&other.col))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_info_creation() {
        let col = ColumnInfo::new(Some("table1".to_string()), "col1".to_string(), None, 0, 0);

        assert_eq!(col.table_name, Some("table1".to_string()));
        assert_eq!(col.column_name, "col1");
        assert_eq!(col.row, 1); // 0-indexed to 1-indexed
        assert_eq!(col.col, 1);
    }

    #[test]
    fn test_column_info_ordering() {
        let col1 = ColumnInfo::new(Some("t1".to_string()), "c1".to_string(), None, 1, 5);
        let col2 = ColumnInfo::new(Some("t1".to_string()), "c2".to_string(), None, 1, 10);
        let col3 = ColumnInfo::new(Some("t1".to_string()), "c3".to_string(), None, 2, 5);

        assert!(col1 < col2);
        assert!(col2 < col3);
    }

    #[test]
    fn test_column_info_display() {
        let col = ColumnInfo::new(
            Some("users".to_string()),
            "user_id".to_string(),
            None,
            5,
            10,
        );

        assert_eq!(format!("{}", col), "users:user_id:6:11");
    }
}
