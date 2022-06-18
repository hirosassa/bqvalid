select
  *
from
  dataset.table
where
  _table_suffix between (
    select dt from dates
  )
  and '2022-06-01'
