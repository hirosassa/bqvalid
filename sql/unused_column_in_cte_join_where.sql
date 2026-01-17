-- Test case: Columns used only in JOIN conditions and WHERE clauses should not be marked as unused
-- This tests that columns referenced in ON clauses and WHERE conditions are correctly tracked
-- No columns should be marked as unused in this case
with
  source_data as (
    select
      id,
      company_id,
      user_id,
      name
    from
      base_table
  ),
  filtered_data as (
    select
      sd.id,
      sd.name
    from
      source_data as sd
    inner join
      external_table as et
    on
      sd.company_id = et.company_id
      and sd.user_id = et.user_id
  )
select
  *
from
  filtered_data
