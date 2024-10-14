use std::fmt::Display;

#[derive(Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub(crate) table_name: Option<String>,
    pub(crate) column_name: String,
    pub(crate) row: usize,
    pub(crate) col: usize,
}

impl ColumnInfo {
    pub(crate) const fn new(
        table_name: Option<String>,
        column_name: String,
        row: usize,
        col: usize,
    ) -> Self {
        Self {
            table_name,
            column_name,
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
