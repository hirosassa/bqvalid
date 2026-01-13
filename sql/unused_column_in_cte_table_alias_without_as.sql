-- Test case: Table aliases without AS keyword
-- This tests that table aliases without AS keyword (e.g., "from source_data sd")
-- are correctly recognized when resolving column references
-- id and name should be marked as unused, while category and value are used
with
  source_data as (
    select
      id,
      name,
      category,
      value
    from
      base_table
  ),
  filtered_data as (
    select
      category,
      value
    from
      source_data sd
  )
select
  fd.category,
  fd.value
from
  filtered_data fd
order by
  category
