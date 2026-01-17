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
