-- Test case: Multiple column aliases across CTEs
-- Testing if aliased columns are correctly traced through multiple CTEs
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
