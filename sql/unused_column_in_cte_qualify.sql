-- Test case: Columns used in QUALIFY clause with SELECT * should not be marked as unused
-- This tests that columns used in QUALIFY clause (BigQuery feature)
-- are correctly recognized as used, especially with SELECT * in intermediate CTE
-- Only unused_field should be marked as unused
with
  source_data as (
    select
      id,
      category,
      value,
      unused_field
    from
      base_table
  ),
  merged_data as (
    select
      id,
      category,
      value,
      concat(id, '-', category) as composite_key
    from
      source_data
  ),
  final as (
    select
      *
    from
      merged_data
    qualify
      row_number() over(partition by composite_key) = 1
  )
select
  *
from
  final
