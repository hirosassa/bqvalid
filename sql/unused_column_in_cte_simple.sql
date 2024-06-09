with data1 as (
  select
    column1,
    column2,
    unused_column1
  from
    table1
), data2 as (
  select
    column3,
    unused_column2
  from
    table2
), data3 as (
  select
    column1,
    column2,
    column3
  from
    data1
  left outer join
    data2
  on
    data1.column1 = data2.column3
)
select 
  * 
from
  data3
