-- Test case: Table aliases with AS keyword and JOIN
-- This tests that table aliases with AS keyword (e.g., "from table1 as t1")
-- are correctly recognized when used in JOIN conditions
with
  orders as (
    select
      order_id,
      customer_id,
      order_date,
      amount
    from
      order_source
  ),
  customers as (
    select
      customer_id,
      customer_name,
      region
    from
      customer_source
  ),
  result as (
    select
      ord.order_id,
      ord.order_date,
      ord.amount,
      cust.customer_name,
      cust.region
    from
      orders as ord
      left join customers as cust on ord.customer_id = cust.customer_id
  )
select
  *
from
  result
