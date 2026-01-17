-- Test case for aggregate functions in final SELECT
-- active_count and inactive_count should NOT be marked as unused
-- because they are used in sum() in the final SELECT
with base as (
  select
    user_id,
    case
      when status = 'active' then 1
      else 0
    end as active_count,
    case
      when status = 'inactive' then 1
      else 0
    end as inactive_count
  from
    users
)
select
  user_id,
  sum(active_count) as total_active,
  sum(inactive_count) as total_inactive
from
  base
group by
  user_id
