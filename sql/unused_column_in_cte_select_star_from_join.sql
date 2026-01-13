-- Test case: select * from joined table
-- This should NOT report unused columns because select * uses all columns from the joined tables
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

