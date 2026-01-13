-- Test case: Complex scenario with multiple joins and select *
-- Testing if columns used only in join conditions are marked as used
with
  table1 as (
    select
      id,
      name,
      age,
      department_id  -- Used only in join condition
    from
      source_table1
  ),
  table2 as (
    select
      id,
      email,
      country,
      user_id  -- Used only in join condition
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
      join table2 on table1.id = table2.user_id  -- table2.user_id used here
      join table3 on table1.department_id = table3.department_id  -- both department_id used here
  )
select
  *
from
  joined_table
