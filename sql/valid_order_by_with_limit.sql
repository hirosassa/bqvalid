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
  top_users

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
)

-- Valid: ORDER BY in final SELECT
select
  id,
  name
from
  table1
order by id
