select
  current_date,
  column_a
from
  dataset.table
where
  _table_suffix between '2022-06-01'
  and (
    select dt from dates
  )