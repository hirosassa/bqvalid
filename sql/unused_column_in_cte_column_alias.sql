-- Test case: Column alias/rename in CTE
-- column1 is renamed to unique_id in cte1
-- unique_id is used in final SELECT with GROUP BY
-- column2 should be reported as unused
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
