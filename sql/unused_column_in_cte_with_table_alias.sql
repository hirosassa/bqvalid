with data1 as (
  select
    column1,
    column2,
    unused_column1
  from
    table1 a
), data2 as (
  select
    *
  from
    data1
)
select
  a.column1,
  a.column2
from
  data2 a
  inner join
    table2 b
  on
    a.column1 = b.column1
  inner join
    table3 c
  on
    a.column1 = c.column1
