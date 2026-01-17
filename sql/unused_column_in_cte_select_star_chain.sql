-- Test case: Multiple SELECT * in chain
-- This tests that when CTEs use SELECT * to reference other CTEs,
-- all columns are correctly tracked and not marked as unused
-- No columns should be marked as unused in this case
with
  source_data as (
    select
      id,
      name,
      category,
      created_at
    from
      base_table
  ),
  intermediate as (
    select
      *
    from
      source_data
  ),
  final_data as (
    select
      *
    from
      intermediate
  )
select
  *
from
  final_data
