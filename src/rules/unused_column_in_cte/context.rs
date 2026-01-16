use std::collections::HashMap;

use super::graph::DependencyGraph;
use super::models::ColumnInfo;

/// Analysis context shared across all visitors
/// Holds the SQL text, dependency graph, and CTE definitions
pub struct AnalysisContext<'a> {
    /// SQL source text
    sql: &'a str,
    /// Dependency graph tracking column usage
    pub graph: DependencyGraph,
    /// CTE column definitions (CTE name -> columns)
    /// Kept for quick lookups by visitors
    pub cte_columns: HashMap<String, Vec<ColumnInfo>>,
}

impl<'a> AnalysisContext<'a> {
    /// Create a new analysis context
    pub fn new(sql: &'a str) -> Self {
        Self {
            sql,
            graph: DependencyGraph::new(),
            cte_columns: HashMap::new(),
        }
    }

    /// Get SQL source text
    pub const fn sql(&self) -> &'a str {
        self.sql
    }

    /// Add a CTE definition to the context
    pub fn add_cte(&mut self, cte_name: String, columns: Vec<ColumnInfo>) {
        self.graph.add_cte(cte_name.clone(), columns.clone());
        self.cte_columns.insert(cte_name, columns);
    }

    /// Mark a column as used
    pub fn mark_used(&mut self, table_name: &str, column_name: &str) {
        self.graph.mark_column_used(table_name, column_name);
    }

    /// Check if a CTE exists
    pub fn has_cte(&self, cte_name: &str) -> bool {
        self.cte_columns.contains_key(cte_name)
    }

    /// Get columns for a specific CTE
    pub fn get_cte_columns(&self, cte_name: &str) -> Option<&Vec<ColumnInfo>> {
        self.cte_columns.get(cte_name)
    }

    /// Collect all unused columns
    pub fn collect_unused(&self) -> Vec<ColumnInfo> {
        self.graph.collect_unused_columns()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let sql = "SELECT * FROM table1";
        let context = AnalysisContext::new(sql);

        assert_eq!(context.sql(), sql);
        assert_eq!(context.cte_columns.len(), 0);
    }

    #[test]
    fn test_add_cte() {
        let sql = "WITH cte1 AS (SELECT col1 FROM t1) SELECT * FROM cte1";
        let mut context = AnalysisContext::new(sql);

        let columns = vec![ColumnInfo::new(
            Some("cte1".to_string()),
            "col1".to_string(),
            None,
            0,
            0,
        )];

        context.add_cte("cte1".to_string(), columns);

        assert!(context.has_cte("cte1"));
        assert!(!context.has_cte("cte2"));
    }

    #[test]
    fn test_mark_used() {
        let sql = "WITH cte1 AS (SELECT col1, col2 FROM t1) SELECT col1 FROM cte1";
        let mut context = AnalysisContext::new(sql);

        let columns = vec![
            ColumnInfo::new(Some("cte1".to_string()), "col1".to_string(), None, 0, 0),
            ColumnInfo::new(Some("cte1".to_string()), "col2".to_string(), None, 0, 5),
        ];

        context.add_cte("cte1".to_string(), columns);
        context.mark_used("cte1", "col1");

        let unused = context.collect_unused();
        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].column_name, "col2");
    }

    #[test]
    fn test_get_cte_columns() {
        let sql = "WITH cte1 AS (SELECT col1 FROM t1) SELECT * FROM cte1";
        let mut context = AnalysisContext::new(sql);

        let columns = vec![ColumnInfo::new(
            Some("cte1".to_string()),
            "col1".to_string(),
            None,
            0,
            0,
        )];

        context.add_cte("cte1".to_string(), columns);

        let retrieved = context.get_cte_columns("cte1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().len(), 1);
        assert_eq!(retrieved.unwrap()[0].column_name, "col1");
    }
}
