select
  *
from
  dataset.table
where
  _table_suffix  = (
    select dt from dates
  )

