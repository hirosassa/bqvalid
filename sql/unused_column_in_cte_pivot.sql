-- Test case: Columns used in PIVOT clause should not be marked as unused
-- This tests that:
-- 1. Columns in aggregate functions (e.g., sum(value)) are recognized as used
-- 2. Columns in FOR clause (input_column) are recognized as used
-- No columns should be marked as unused in this case
with raw_data as (
  select
    category,
    month,
    value
  from
    source_table
),
pivoted as (
  select
    category,
    jan,
    feb,
    mar
  from
    raw_data
  pivot(
    sum(value)
    for month in ('Jan' as jan, 'Feb' as feb, 'Mar' as mar)
  )
)
select * from pivoted
