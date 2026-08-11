use crate::ast::NodeRef;

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::helpers::{find_child_of_kind, has_child_of_kind, one_based_start};
use crate::rules::rule::Rule;

const RULE_ID: &str = "unnecessary_order_by";

/// Flags `ORDER BY` in a CTE or subquery without `LIMIT`, where the sort has no
/// effect and only wastes work.
pub struct UnnecessaryOrderBy;

impl Rule for UnnecessaryOrderBy {
    fn id(&self) -> &'static str {
        RULE_ID
    }

    fn check_node(&self, node: NodeRef<'_>, sql: &str, diagnostics: &mut Vec<Diagnostic>) {
        if node.kind() == "ASTQuery"
            && let Some(diagnostic) = check_unnecessary_order_by_in_scope(&node, sql)
        {
            diagnostics.push(diagnostic);
        }
    }
}

/// The query body to inspect for an unnecessary ORDER BY, plus the kind names of
/// its ORDER BY and LIMIT children.
///
/// On googlesql the scope is an `ASTQuery` (nested inside a CTE entry or a
/// subquery) whose direct children are the `ASTOrderBy` / `ASTLimitOffset`; the
/// top-level query is not a scope, so its trailing ORDER BY (which does have an
/// effect) is left alone.
fn scope_query_body<'a>(scope: &NodeRef<'a>) -> Option<(NodeRef<'a>, &'static str, &'static str)> {
    match scope.kind() {
        "ASTQuery" if is_nested_query(scope) => Some((*scope, "ASTOrderBy", "ASTLimitOffset")),
        _ => None,
    }
}

/// True when an `ASTQuery` is nested inside a CTE entry or a subquery rather than
/// being the statement's top-level query.
fn is_nested_query(query: &NodeRef<'_>) -> bool {
    query.parent().is_some_and(|parent| {
        matches!(
            parent.kind(),
            "ASTAliasedQuery" | "ASTTableSubquery" | "ASTExpressionSubquery"
        )
    })
}

fn check_unnecessary_order_by_in_scope(scope_node: &NodeRef<'_>, _sql: &str) -> Option<Diagnostic> {
    let (query_body, order_by_kind, limit_kind) = scope_query_body(scope_node)?;

    if !has_child_of_kind(&query_body, limit_kind)
        && let Some(order_by_node) = find_child_of_kind(&query_body, order_by_kind)
    {
        let (row, col) = one_based_start(&order_by_node);
        return Some(Diagnostic::new(
            RULE_ID,
            Severity::Warning,
            row,
            col,
            "Unnecessary ORDER BY: This ORDER BY clause has no effect without LIMIT/OFFSET or in aggregate functions".to_string(),
        ));
    }

    None
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
    use crate::rules::helpers::run_rule;
    use rstest::rstest;

    #[rstest]
    #[case(
        "\
-- Unnecessary ORDER BY in CTE
with sorted_data as (
  select
    id,
    name
  from
    table1
  order by id  -- This ORDER BY is ignored
)
select
  *
from
  sorted_data
",
        1
    )]
    #[case(
        "\
-- Unnecessary ORDER BY in subquery
select
  *
from (
  select
    id,
    name
  from
    table1
  order by id  -- This ORDER BY is ignored
)
where
  name = 'test'
",
        1
    )]
    fn test_unnecessary_order_by_exists(#[case] sql: &str, #[case] expected_count: usize) {
        let diagnostics = run_rule(&UnnecessaryOrderBy, sql);
        assert_eq!(diagnostics.len(), expected_count);
    }

    #[test]
    fn test_valid_order_by_with_limit() {
        // ORDER BY + LIMIT in a CTE, ORDER BY + LIMIT in a subquery, and a
        // trailing ORDER BY in the final SELECT are all valid. googlesql requires
        // `;` between the three statements.
        let sql = "\
-- Valid: ORDER BY with LIMIT in CTE
with top_users as (
  select
    id,
    name
  from
    table1
  order by id
  limit 10
)
select
  *
from
  top_users;

-- Valid: ORDER BY with LIMIT in subquery
select
  *
from (
  select
    id,
    name
  from
    table1
  order by id
  limit 10
);

-- Valid: ORDER BY in final SELECT
select
  id,
  name
from
  table1
order by id;
";
        // parse_sql handles a single statement; drive each of the three here.
        for statement in sql.split(';').filter(|s| !s.trim().is_empty()) {
            let diagnostics = run_rule(&UnnecessaryOrderBy, statement);
            assert!(
                diagnostics.is_empty(),
                "unexpected diagnostics for: {statement}"
            );
        }
    }

    #[test]
    fn limit_or_final_order_by_is_not_flagged() {
        // Adding LIMIT to a CTE ORDER BY, or a top-level trailing ORDER BY, is not
        // flagged.
        let cte_with_limit = "WITH s AS (SELECT id FROM t ORDER BY id LIMIT 10) SELECT * FROM s";
        let final_order_by = "SELECT id FROM t ORDER BY id";
        assert!(run_rule(&UnnecessaryOrderBy, cte_with_limit).is_empty());
        assert!(run_rule(&UnnecessaryOrderBy, final_order_by).is_empty());
    }

    #[test]
    fn points_at_the_order_by_position() {
        let sql = "WITH s AS (SELECT id FROM t ORDER BY id) SELECT * FROM s";
        let order_col = sql.find("ORDER").expect("query contains ORDER BY");
        let diagnostics = run_rule(&UnnecessaryOrderBy, sql);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].row(), 1);
        assert_eq!(diagnostics[0].col(), order_col + 1);
    }
}
