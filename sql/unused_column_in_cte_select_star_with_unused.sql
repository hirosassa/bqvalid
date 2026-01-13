-- Test case: select * from joined table with unused columns
-- This SHOULD report unused columns in table1 and table2 that are not selected in joined_table
with
  table1 as (
    select
      id,
      name,
      age,
      unused_field1  -- This should be reported as unused
    from
      source_table1
  ),
  table2 as (
    select
      id,
      email,
      country,
      unused_field2  -- This should be reported as unused
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
