-- Test case for UNNEST in FROM clause
-- date_array should NOT be marked as unused
-- because it is used in unnest(date_array) in the final SELECT
with date_array_cte as (
  select
    user_id,
    generate_date_array(start_date, end_date) as date_array
  from
    users
)
select
  user_id,
  date
from
  date_array_cte,
  unnest(date_array) as date
