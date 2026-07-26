use tree_sitter::{Node, Tree};
use tree_sitter_traversal::{Order, traverse};

use crate::diagnostic::Diagnostic;
use crate::rules::{
    compare_table_suffix_with_subquery::CompareTableSuffixWithSubquery,
    invalid_group_by::InvalidGroupBy, unnecessary_order_by::UnnecessaryOrderBy,
    unused_column_in_cte::UnusedColumnInCte, use_current_date::UseCurrentDate,
};

/// A single lint rule.
///
/// Rules come in two shapes. Most react to individual syntax nodes and
/// implement [`Rule::check_node`], which the shared traversal in [`run_rules`]
/// calls once per node so every rule sees the tree in a single pre-order pass.
/// Rules that need cross-node analysis (e.g. tracking CTE columns across the
/// whole query) instead implement [`Rule::check_tree`] and walk the tree
/// themselves.
pub trait Rule {
    /// Stable identifier for the rule. Used to reference the rule in output and
    /// (in the future) in configuration and inline suppression.
    fn id(&self) -> &'static str;

    /// React to a single node visited during the shared pre-order traversal.
    /// Node-driven rules override this; the default does nothing so tree-driven
    /// rules can ignore it.
    fn check_node(&self, _node: Node<'_>, _sql: &str, _diagnostics: &mut Vec<Diagnostic>) {}

    /// React to the whole tree, for rules that cannot be expressed per node.
    /// The default does nothing so node-driven rules can ignore it.
    fn check_tree(&self, _tree: &Tree, _sql: &str, _diagnostics: &mut Vec<Diagnostic>) {}

    /// Run this rule alone over `tree`. Convenience for unit tests and callers
    /// that want a single rule's diagnostics; [`run_rules`] shares one traversal
    /// across every rule instead.
    fn check(&self, tree: &Tree, sql: &str) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();
        for node in traverse(tree.walk(), Order::Pre) {
            self.check_node(node, sql, &mut diagnostics);
        }
        self.check_tree(tree, sql, &mut diagnostics);
        diagnostics
    }
}

/// The registry of every enabled rule. This is the single place rules are wired
/// in: adding a rule means adding one entry here rather than editing the
/// analysis loop.
pub fn all_rules() -> Vec<Box<dyn Rule>> {
    vec![
        Box::new(CompareTableSuffixWithSubquery),
        Box::new(InvalidGroupBy),
        Box::new(UnnecessaryOrderBy),
        Box::new(UnusedColumnInCte),
        Box::new(UseCurrentDate),
    ]
}

/// Run all registered rules over `tree` in a single pre-order traversal.
///
/// Node-driven rules each inspect every node during the one shared pass, rather
/// than each rule walking the whole tree on its own. Tree-driven rules run
/// afterwards. Diagnostics come out in traversal order for the node-driven
/// rules, followed by the tree-driven ones.
pub fn run_rules(tree: &Tree, sql: &str) -> Vec<Diagnostic> {
    let rules = all_rules();
    let mut diagnostics = Vec::new();

    for node in traverse(tree.walk(), Order::Pre) {
        for rule in &rules {
            rule.check_node(node, sql, &mut diagnostics);
        }
    }
    for rule in &rules {
        rule.check_tree(tree, sql, &mut diagnostics);
    }

    diagnostics
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
    use crate::rules::helpers::parse_sql;
    use std::collections::HashSet;

    #[test]
    fn all_rules_have_unique_non_empty_ids() {
        let rules = all_rules();
        assert_eq!(rules.len(), 5, "every rule must be registered");

        let ids: HashSet<&str> = rules.iter().map(|r| r.id()).collect();
        assert_eq!(ids.len(), rules.len(), "rule ids must be unique");
        assert!(rules.iter().all(|r| !r.id().is_empty()), "ids must be set");
    }

    #[test]
    fn run_rules_aggregates_multiple_rules_in_a_single_pass() {
        // One query that trips two independent node-driven rules. run_rules must
        // surface both from its single shared traversal.
        let sql = "SELECT CURRENT_DATE() AS d \
                   FROM t \
                   WHERE _TABLE_SUFFIX = (SELECT MAX(suffix) FROM u)";
        let tree = parse_sql(sql);
        let diagnostics = run_rules(&tree, sql);

        assert!(
            diagnostics
                .iter()
                .any(|d| d.message().contains("CURRENT_DATE")),
            "expected a CURRENT_DATE diagnostic"
        );
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message().contains("Full scan")),
            "expected a full-scan diagnostic"
        );
    }

    #[test]
    fn run_rules_matches_each_rules_own_output() {
        // run_rules' single pass must produce exactly the union of what each rule
        // produces on its own, so merging traversals changes performance, not
        // behaviour.
        let sql = "SELECT CURRENT_DATE() AS d \
                   FROM t \
                   WHERE _TABLE_SUFFIX = (SELECT MAX(suffix) FROM u)";
        let tree = parse_sql(sql);

        let combined: HashSet<String> = run_rules(&tree, sql)
            .iter()
            .map(ToString::to_string)
            .collect();

        let mut expected: HashSet<String> = HashSet::new();
        for rule in all_rules() {
            expected.extend(rule.check(&tree, sql).iter().map(ToString::to_string));
        }

        assert_eq!(combined, expected);
    }

    #[test]
    fn run_rules_on_clean_query_yields_nothing() {
        let sql = "SELECT id, name FROM users";
        let tree = parse_sql(sql);
        assert!(run_rules(&tree, sql).is_empty());
    }
}
