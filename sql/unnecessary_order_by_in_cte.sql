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
