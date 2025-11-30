with data1 as (
  select
    id,
    name,
    unused_field1,
    unused_field2
  from
    table1
), data2 as (
  select
    id,
    amount,
    unused_amount_field
  from
    table2
), data3 as (
  select
    id,
    price,
    unused_price_field,
    another_unused
  from
    table3
), joined_data as (
  select
    data1.id,
    data1.name,
    data2.amount,
    data3.price
  from
    data1
  inner join
    data2
  on
    data1.id = data2.id
  inner join
    data3
  on
    data1.id = data3.id
)
select
  *
from
  joined_data
