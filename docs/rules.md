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
