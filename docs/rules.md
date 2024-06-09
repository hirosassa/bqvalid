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
