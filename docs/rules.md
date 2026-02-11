# Rules

The list of the linting rules are as follows.

## Comparing `_TABLE_SUFFIX` with subquery

Comparing `_TABLE_SUFFIX` pseudo column with dynamic expression like subquery will cause full scan on wildcard tables.

ref: [official code example](https://cloud.google.com/bigquery/docs/querying-wildcard-tables#filter_selected_tables_using_table_suffix)

### Example

```sql
select
  *
from
  dataset.table
where
  _table_suffix between '2022-06-01'
  and (
    select dt from dates
  )

```

## Using CURRENT_DATE

Using `CURRENT_DATE` will make the SQL maintainability worse. Date parameters should be passed by outside of the script.

### Example

```sql
select
  current_date,
  column_a
from
  dataset.table

```

## Contains unused columns in CTE

Unused columns reference in CTE will make the SQL readability worse.

### Example

```sql
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
```

## Unnecessary ORDER BY in CTE or subquery

ORDER BY clauses in CTEs or subqueries have no effect unless they are used with LIMIT/OFFSET or within aggregate functions like ARRAY_AGG. Using ORDER BY without these constructs wastes resources and provides no benefit, as the ordering is not guaranteed to be preserved in the final result.

ref: [BigQuery documentation on query execution](https://cloud.google.com/bigquery/docs/reference/standard-sql/query-syntax#order_by_clause)

### Example

```sql
-- Unnecessary ORDER BY in CTE
WITH sorted_data AS (
  SELECT
    id,
    name
  FROM
    table1
  ORDER BY id  -- This ORDER BY has no effect
)
SELECT
  *
FROM
  sorted_data

-- Unnecessary ORDER BY in subquery
SELECT
  *
FROM (
  SELECT
    id,
    name
  FROM
    table1
  ORDER BY id  -- This ORDER BY has no effect
)
WHERE
  name = 'test'
```

### Valid use cases

ORDER BY is meaningful in the following cases:

```sql
-- Valid: ORDER BY with LIMIT
WITH top_users AS (
  SELECT
    id,
    name
  FROM
    table1
  ORDER BY id
  LIMIT 10  -- ORDER BY is necessary for TOP-N query
)
SELECT * FROM top_users

-- Valid: ORDER BY in ARRAY_AGG
SELECT
  user_id,
  ARRAY_AGG(event_name ORDER BY timestamp) as events  -- ORDER BY controls array order
FROM events
GROUP BY user_id

-- Valid: ORDER BY in final SELECT
SELECT
  id,
  name
FROM
  table1
ORDER BY id  -- ORDER BY in final SELECT controls output order
```

## Invalid GROUP BY usage

When using GROUP BY, all columns in the SELECT clause must either:
1. Appear in the GROUP BY clause, or
2. Be used within an aggregate function

Columns that don't meet either condition will cause a SQL error or produce incorrect results. This rule helps catch these issues early in development.

ref: [BigQuery GROUP BY documentation](https://cloud.google.com/bigquery/docs/reference/standard-sql/query-syntax#group_by_clause)

### Example

```sql
-- Invalid: col2 is not in GROUP BY and not in an aggregate function
SELECT
  col1,
  col2,  -- Error: col2 must be in GROUP BY or aggregated
  COUNT(*) as cnt
FROM
  my_table
GROUP BY col1

-- Invalid: Multiple violations
[SELECT]
  col1,
  col2,  -- Error: not in GROUP BY
  col3,  -- Error: not in GROUP BY
  COUNT(*) as cnt
FROM
  my_table
GROUP BY col1

-- Invalid: Qualified column name not in GROUP BY
SELECT
  t.col1,
  t.col2,  -- Error: t.col2 must be in GROUP BY or aggregated
  COUNT(*) as cnt
FROM
  my_table t
GROUP BY t.col1
```

### Valid use cases

```sql
-- Valid: All non-aggregated columns are in GROUP BY
SELECT
  col1,
  COUNT(col2) as cnt,
  SUM(col3) as total
FROM
  my_table
GROUP BY col1

-- Valid: Multiple columns in GROUP BY
SELECT
  col1,
  col2,
  MAX(col3) as max_val
FROM
  my_table
GROUP BY col1, col2

-- Valid: All aggregate functions are supported
SELECT
  user_id,
  COUNT(*) as cnt,
  AVG(amount) as avg_amount,
  APPROX_COUNT_DISTINCT(product_id) as unique_products,
  STDDEV(price) as price_stddev
FROM
  sales
GROUP BY user_id

-- Valid: Qualified column names
SELECT
  t1.user_id,
  t2.category,
  COUNT(*) as cnt
FROM
  users t1
JOIN
  orders t2
ON
  t1.id = t2.user_id
GROUP BY t1.user_id, t2.category

-- Valid: Query without GROUP BY
SELECT
  col1,
  col2,
  col3
FROM
  my_table
```
