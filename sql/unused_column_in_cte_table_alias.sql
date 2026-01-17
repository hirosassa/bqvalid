-- Test case with table aliases
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
