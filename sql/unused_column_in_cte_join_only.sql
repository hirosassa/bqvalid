with data1 as (
  select
    id,
    name,
    unused_field
  from
    table1
), data2 as (
  select
    id,
    amount
  from
    table2
), joined_data as (
  select
    data1.name,
    data2.amount
  from
    data1
  inner join
    data2
  on
    data1.id = data2.id
)
select
  *
from
  joined_data
