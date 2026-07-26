// Visitor-based implementation modules
mod context;
mod graph;
mod models;
mod utils;
mod visitor;
mod visitors;

use tree_sitter::Tree;
use tree_sitter_traversal::{Order, traverse};

use crate::diagnostic::{Diagnostic, Severity};
use crate::rules::rule::Rule;

use context::AnalysisContext;
use visitor::NodeVisitor;
use visitors::{
    CteVisitor, PivotVisitor, QualifyVisitor, SelectStarVisitor, SelectVisitor, WhereVisitor,
};

const RULE_ID: &str = "unused_column_in_cte";

/// Flags columns defined in a CTE but never referenced afterwards.
///
/// This rule needs cross-node analysis (which CTE columns get used anywhere in
/// the query), so it walks the tree itself via [`Rule::check_tree`] rather than
/// reacting to individual nodes in the shared traversal.
pub struct UnusedColumnInCte;

impl Rule for UnusedColumnInCte {
    fn id(&self) -> &'static str {
        RULE_ID
    }

    fn check_tree(&self, tree: &Tree, sql: &str, diagnostics: &mut Vec<Diagnostic>) {
        diagnostics.extend(check(tree, sql));
    }
}

pub fn check(tree: &Tree, sql: &str) -> Vec<Diagnostic> {
    let mut context = AnalysisContext::new(sql);

    let cte_visitor = CteVisitor;
    let select_star_visitor = SelectStarVisitor;
    let select_visitor = SelectVisitor::new();
    let where_visitor = WhereVisitor;
    let qualify_visitor = QualifyVisitor;
    let pivot_visitor = PivotVisitor;

    // Single-pass traversal with all visitors
    // Note: DistinctVisitor removed - DISTINCT doesn't make all CTE columns used,
    // only the columns in the SELECT clause are affected by DISTINCT
    for node in traverse(tree.root_node().walk(), Order::Pre) {
        cte_visitor.visit(&node, &mut context);
        select_star_visitor.visit(&node, &mut context);
        select_visitor.visit(&node, &mut context);
        where_visitor.visit(&node, &mut context);
        qualify_visitor.visit(&node, &mut context);
        pivot_visitor.visit(&node, &mut context);
    }

    context
        .collect_unused()
        .into_iter()
        .map(|col| {
            Diagnostic::new(
                RULE_ID,
                Severity::Warning,
                col.row,
                col.col,
                format!("Unused column: {}", col.column_name),
            )
        })
        .collect()
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

    /// Each case pairs an inline query with the column names expected to be
    /// reported as unused.
    #[rstest]
    #[case(
        "\
with data1 as (
  select
    column1,
    column2,
    unused_column1
  from
    table1
), data2 as (
  select
    column3,
    unused_column2
  from
    table2
), data3 as (
  select
    column1,
    column2,
    column3
  from
    data1
  left outer join
    data2
  on
    data1.column1 = data2.column3
)
select
  *
from
  data3
",
        vec!["unused_column1", "unused_column2"]
    )]
    // column1 is renamed to unique_id and used; column2 and unused_column are not.
    #[case(
        "\
with
  cte1 as (
    select
      column1 as unique_id,
      column2,
      unused_column
    from
      source_table
  )
select
  unique_id,
  count(*)
from
  cte1
group by
  unique_id
",
        vec!["column2", "unused_column"]
    )]
    // Columns used as window/aggregate function arguments are used; only
    // unused_field is unused.
    #[case(
        "\
with
aggregated_data as (
  select
    category,
    version,
    unused_field,
    count(1) as user_count
  from
    source_table
  group by
    category,
    version
),
cumulative_data as (
  select
    category,
    version,
    user_count,
    sum(user_count) over (
      partition by category
      order by version desc
    ) as cumulative_count
  from
    aggregated_data
),
total_data as (
  select
    category,
    sum(user_count) as total_count
  from
    cumulative_data
  group by
    category
)
select
  cd.category,
  cd.version,
  cd.cumulative_count,
  td.total_count
from
  cumulative_data cd
  inner join total_data td on cd.category = td.category
",
        vec!["unused_field"]
    )]
    #[case(
        "\
with data1 as (
  select
    id,
    name,
    unused_field
  from
    table1
), data2 as (
  select
    id,
    amount
  from
    table2
), joined_data as (
  select
    data1.name,
    data2.amount
  from
    data1
  inner join
    data2
  on
    data1.id = data2.id
)
select
  *
from
  joined_data
",
        vec!["unused_field"]
    )]
    #[case(
        "\
with data1 as (
  select
    id,
    name,
    unused_field1,
    unused_field2
  from
    table1
), data2 as (
  select
    id,
    amount,
    unused_amount_field
  from
    table2
), data3 as (
  select
    id,
    price,
    unused_price_field,
    another_unused
  from
    table3
), joined_data as (
  select
    data1.id,
    data1.name,
    data2.amount,
    data3.price
  from
    data1
  inner join
    data2
  on
    data1.id = data2.id
  inner join
    data3
  on
    data1.id = data3.id
)
select
  *
from
  joined_data
",
        vec![
            "unused_field1",
            "unused_field2",
            "unused_amount_field",
            "unused_price_field",
            "another_unused"
        ]
    )]
    // Aliased columns traced through multiple CTEs.
    #[case(
        "\
with
  cte1 as (
    select
      id as user_id,
      name as user_name,
      email,
      unused_field1
    from
      users_table
  ),
  cte2 as (
    select
      user_id as uid,
      user_name,
      unused_field2
    from
      cte1
  )
select
  uid,
  user_name
from
  cte2
",
        vec!["email", "unused_field1", "unused_field2"]
    )]
    // Table aliases without AS keyword; id and name are unused.
    #[case(
        "\
with
  source_data as (
    select
      id,
      name,
      category,
      value
    from
      base_table
  ),
  filtered_data as (
    select
      category,
      value
    from
      source_data sd
  )
select
  fd.category,
  fd.value
from
  filtered_data fd
order by
  category
",
        vec!["id", "name"]
    )]
    #[case(
        "\
with
dim_contract as (
  select distinct -- コメント
    team_id
    , start_date
    , end_date
    , contract_type
    , base_fee
    , account_fee
    , free_account_count
  from `project`.`dataset`.`dim_contract`
  where
    team_id is not null
    and start_date is not null
    and contract_type = '無償提供'
)

select
  team_id,
  start_date,
  end_date
from
  dim_contract
",
        vec!["contract_type", "base_fee", "account_fee", "free_account_count"]
    )]
    // Columns used in QUALIFY with SELECT * are used; only unused_field is unused.
    #[case(
        "\
with
  source_data as (
    select
      id,
      category,
      value,
      unused_field
    from
      base_table
  ),
  merged_data as (
    select
      id,
      category,
      value,
      concat(id, '-', category) as composite_key
    from
      source_data
  ),
  final as (
    select
      *
    from
      merged_data
    qualify
      row_number() over(partition by composite_key) = 1
  )
select
  *
from
  final
",
        vec!["unused_field"]
    )]
    // select * from a joined table still reports the columns not selected downstream.
    #[case(
        "\
with
  table1 as (
    select
      id,
      name,
      age,
      unused_field1
    from
      source_table1
  ),
  table2 as (
    select
      id,
      email,
      country,
      unused_field2
    from
      source_table2
  ),
  joined_table as (
    select
      table1.id,
      table1.name,
      table1.age,
      table2.email,
      table2.country
    from
      table1
      join table2 on table1.id = table2.id
  )
select
  *
from
  joined_table
",
        vec!["unused_field1", "unused_field2"]
    )]
    // Columns used only in join conditions are used; id is unused.
    #[case(
        "\
with
  table1 as (
    select
      id,
      name,
      age,
      department_id
    from
      source_table1
  ),
  table2 as (
    select
      id,
      email,
      country,
      user_id
    from
      source_table2
  ),
  table3 as (
    select
      department_id,
      department_name
    from
      source_table3
  ),
  joined_table as (
    select
      table1.id,
      table1.name,
      table1.age,
      table2.email,
      table2.country,
      table3.department_name
    from
      table1
      join table2 on table1.id = table2.user_id
      join table3 on table1.department_id = table3.department_id
  )
select
  *
from
  joined_table
",
        vec!["id"]
    )]
    fn test_integration_with_sql_files(#[case] sql: &str, #[case] expected_unused: Vec<&str>) {
        let diagnostics = run_rule(&UnusedColumnInCte, sql);

        assert!(
            !diagnostics.is_empty(),
            "Expected unused columns, but found none"
        );

        let mut found_columns: Vec<String> = diagnostics
            .iter()
            .map(|d| {
                // Extract column name from "Unused column: <name>" message
                d.message()
                    .strip_prefix("Unused column: ")
                    .unwrap_or_else(|| d.message())
                    .to_string()
            })
            .collect();

        found_columns.sort();
        let mut expected_sorted = expected_unused
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>();
        expected_sorted.sort();

        assert_eq!(
            found_columns, expected_sorted,
            "Unused columns mismatch.\nExpected: {:?}\nFound: {:?}",
            expected_sorted, found_columns
        );
    }

    /// Queries that should report no unused columns.
    #[rstest]
    // active_count/inactive_count are used via sum() in the final SELECT.
    #[case(
        "\
with base as (
  select
    user_id,
    case
      when status = 'active' then 1
      else 0
    end as active_count,
    case
      when status = 'inactive' then 1
      else 0
    end as inactive_count
  from
    users
)
select
  user_id,
  sum(active_count) as total_active,
  sum(inactive_count) as total_inactive
from
  base
group by
  user_id
"
    )]
    // Columns used only in JOIN/WHERE conditions are tracked as used.
    #[case(
        "\
with
  source_data as (
    select
      id,
      company_id,
      user_id,
      name
    from
      base_table
  ),
  filtered_data as (
    select
      sd.id,
      sd.name
    from
      source_data as sd
    inner join
      external_table as et
    on
      sd.company_id = et.company_id
      and sd.user_id = et.user_id
  )
select
  *
from
  filtered_data
"
    )]
    #[case(
        "\
with
  table1 as (
    select
      id,
      name
    from
      source
  ),
  table2 as (
    select
      t1.id,
      t1.name
    from
      table1 as t1
  )
select
  *
from
  table2
"
    )]
    // Table aliases with AS keyword used in JOIN conditions.
    #[case(
        "\
with
  orders as (
    select
      order_id,
      customer_id,
      order_date,
      amount
    from
      order_source
  ),
  customers as (
    select
      customer_id,
      customer_name,
      region
    from
      customer_source
  ),
  result as (
    select
      ord.order_id,
      ord.order_date,
      ord.amount,
      cust.customer_name,
      cust.region
    from
      orders as ord
      left join customers as cust on ord.customer_id = cust.customer_id
  )
select
  *
from
  result
"
    )]
    // date_array is used via unnest() in the final SELECT.
    #[case(
        "\
with date_array_cte as (
  select
    user_id,
    generate_date_array(start_date, end_date) as date_array
  from
    users
)
select
  user_id,
  date
from
  date_array_cte,
  unnest(date_array) as date
"
    )]
    // Columns used in PIVOT (aggregate arg and FOR clause) are used.
    #[case(
        "\
with raw_data as (
  select
    category,
    month,
    value
  from
    source_table
),
pivoted as (
  select
    category,
    jan,
    feb,
    mar
  from
    raw_data
  pivot(
    sum(value)
    for month in ('Jan' as jan, 'Feb' as feb, 'Mar' as mar)
  )
)
select * from pivoted
"
    )]
    // SELECT * chained across CTEs tracks all columns as used.
    #[case(
        "\
with
  source_data as (
    select
      id,
      name,
      category,
      created_at
    from
      base_table
  ),
  intermediate as (
    select
      *
    from
      source_data
  ),
  final_data as (
    select
      *
    from
      intermediate
  )
select
  *
from
  final_data
"
    )]
    // select * from a joined table uses all columns from both sides.
    #[case(
        "\
with
  table1 as (
    select
      id,
      name,
      age
    from
      source_table1
  ),
  table2 as (
    select
      id,
      email,
      country
    from
      source_table2
  ),
  joined_table as (
    select
      table1.id,
      table1.name,
      table1.age,
      table2.email,
      table2.country
    from
      table1
      join table2 on table1.id = table2.id
  )
select
  *
from
  joined_table
"
    )]
    fn test_integration_no_unused_columns(#[case] sql: &str) {
        let diagnostics = run_rule(&UnusedColumnInCte, sql);

        assert!(
            diagnostics.is_empty(),
            "Expected no unused columns, but found: {:?}",
            diagnostics
                .iter()
                .map(|diag| diag.message().to_string())
                .collect::<Vec<_>>()
        );
    }
}
