-- Test case: Columns used as function arguments should not be marked as unused
-- This tests that columns used in:
-- 1. Window function arguments: sum(user_count) over (...)
-- 2. Aggregate function arguments: sum(user_count)
-- are correctly recognized as used, not unused
-- Only unused_field should be marked as unused
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
