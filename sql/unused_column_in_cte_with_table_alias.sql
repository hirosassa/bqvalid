with data1 as (
  select
    column_1,
    column_2,
    unused_column_1
  from
    table1 a
), data2 as (
  select
    *
  from
    data1
)
select
  a.column_1,
  a.column_2
from
  data2 a
  inner join
    table2 b
  on
    a.column_1 = b.column_1
  inner join
    table3 c
  on
    a.column_1 = c.column_1
