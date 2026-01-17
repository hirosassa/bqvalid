use std::collections::{HashMap, HashSet};

use super::models::ColumnInfo;
use super::utils::extract_column_name;

/// CTE node in the dependency graph
#[derive(Debug, Clone)]
pub struct CTENode {
    pub columns: Vec<ColumnInfo>,
    pub used_column_names: HashSet<String>,
}

/// Dependency graph managing CTE column usage
pub struct DependencyGraph {
    nodes: HashMap<String, CTENode>,
}

impl DependencyGraph {
    /// Create a new empty dependency graph
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    /// Add a CTE with its columns to the graph
    pub fn add_cte(&mut self, cte_name: &str, columns: &[ColumnInfo]) {
        let node = CTENode {
            columns: columns.to_vec(),
            used_column_names: HashSet::new(),
        };
        self.nodes.insert(cte_name.to_string(), node);
    }

    /// Mark a column as used in a specific CTE
    pub fn mark_column_used(&mut self, table_name: &str, column_name: &str) {
        if let Some(node) = self.nodes.get_mut(table_name) {
            node.used_column_names.insert(column_name.to_string());
        }
    }

    /// Check if a column is used in a specific CTE
    pub fn is_column_used(&self, table_name: &str, column_name: &str) -> bool {
        self.nodes
            .get(table_name)
            .is_some_and(|node| node.used_column_names.contains(column_name))
    }

    /// Collect all unused columns across all CTEs
    pub fn collect_unused_columns(&self) -> Vec<ColumnInfo> {
        let mut unused = Vec::new();

        for cte_node in self.nodes.values() {
            for col in &cte_node.columns {
                let col_base_name = extract_column_name(&col.column_name);
                let is_used = cte_node.used_column_names.contains(&col.column_name)
                    || cte_node
                        .used_column_names
                        .iter()
                        .any(|used| extract_column_name(used) == col_base_name);

                if !is_used {
                    unused.push(col.clone());
                }
            }
        }

        unused.sort();
        unused
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependency_graph_new() {
        let graph = DependencyGraph::new();
        assert_eq!(graph.nodes.len(), 0);
    }

    #[test]
    fn test_add_cte() {
        let mut graph = DependencyGraph::new();

        let columns = vec![
            ColumnInfo::new(Some("cte1".to_string()), "col1".to_string(), None, 0, 0),
            ColumnInfo::new(Some("cte1".to_string()), "col2".to_string(), None, 0, 5),
        ];

        graph.add_cte("cte1", &columns);

        assert_eq!(graph.nodes.len(), 1);
        assert!(graph.nodes.contains_key("cte1"));
    }

    #[test]
    fn test_mark_column_used() {
        let mut graph = DependencyGraph::new();

        let columns = vec![ColumnInfo::new(
            Some("cte1".to_string()),
            "col1".to_string(),
            None,
            0,
            0,
        )];

        graph.add_cte("cte1", &columns);
        graph.mark_column_used("cte1", "col1");

        assert!(graph.is_column_used("cte1", "col1"));
        assert!(!graph.is_column_used("cte1", "col2"));
    }

    #[test]
    fn test_collect_unused_columns() {
        let mut graph = DependencyGraph::new();

        let columns = vec![
            ColumnInfo::new(Some("cte1".to_string()), "col1".to_string(), None, 0, 0),
            ColumnInfo::new(Some("cte1".to_string()), "col2".to_string(), None, 0, 5),
            ColumnInfo::new(Some("cte1".to_string()), "col3".to_string(), None, 0, 10),
        ];

        graph.add_cte("cte1", &columns);

        // Mark col1 and col2 as used
        graph.mark_column_used("cte1", "col1");
        graph.mark_column_used("cte1", "col2");

        let unused = graph.collect_unused_columns();

        assert_eq!(unused.len(), 1);
        assert_eq!(unused[0].column_name, "col3");
    }

    #[test]
    fn test_collect_unused_with_qualified_names() {
        let mut graph = DependencyGraph::new();

        let columns = vec![ColumnInfo::new(
            Some("cte1".to_string()),
            "cte1.col1".to_string(),
            None,
            0,
            0,
        )];

        graph.add_cte("cte1", &columns);

        // Mark using base name
        graph.mark_column_used("cte1", "col1");

        let unused = graph.collect_unused_columns();
        assert_eq!(unused.len(), 0);
    }
}
