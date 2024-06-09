# bqvalid

[![build](https://github.com/hirosassa/bqvalid/actions/workflows/test.yaml/badge.svg)](https://github.com/hirosassa/bqvalid/actions/workflows/test.yaml)
[![codecov](https://codecov.io/gh/hirosassa/bqvalid/branch/main/graph/badge.svg?token=Q5FIA58YTN)](https://codecov.io/gh/hirosassa/bqvalid)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/hirosassa/bqvalid/blob/main/LICENSE)

## What bqvalid does

`bqvalid` is a SQL linter tool for BigQuery GoogleSQL (formerly known as StandardSQL).
`bqvalid` fails with error message if there are the violation of rules described in the next subsections.

### Rules

#### Comparing `_TABLE_SUFFIX` with subquery

Comparing `_TABLE_SUFFIX` pseudo column with dynamic expression like subquery will cause full scan on wildcard tables.
[official code example](https://cloud.google.com/bigquery/docs/querying-wildcard-tables#filter_selected_tables_using_table_suffix)


#### Using CURRENT_DATE

Using `CURRENT_DATE` will make the SQL maintainability worse. Date parameters should be passed by outside of the script.


#### Contains unused columns in CTE

Unused columns reference in CTE will make the SQL readability worse.


## Usage

```shell
cat sample.sql | bqvalid
```

If the SQL is contained the expressions that comparing `_TABLE_SUFFIX` with subquery, `bqvalid` outputs the reason and its position like:
```
5:7: Full scan will cause! Should not compare _TABLE_SUFFIX with subquery
```

Also, you can input file paths or directory. `bqvalid` collects files whose extension is `.sql` (ignores files that has other extensions) :

```shell
bqvalid one.sql two.sql three.sql
```

or
```shell
bqvalid sql/
```

Then, the output will as follows:
```
one.sql:6:6: Full scan will cause! Should not compare _TABLE_SUFFIX with subquery
three.sql:5:19: Full scan will cause! Should not compare _TABLE_SUFFIX with subquery
```

## Contributing

We welcome code contributions for new features and bug fixes!

If you want to add new linting rules, use the following steps:

1. Check the [issues page](https://github.com/hirosassa/bqvalid/issues) on GitHub to see if the task you want to complete is listed there.
1. Create an issue branch for your local work.
1. Add your code in `src/rules/` and implement `pub fn check(tree &Tree, sql: &str)` function in it.
1. Call your new rules from `analyse_sql` function in `src/main.rs`.
1. Write unit tests for your code and make sure everything is still working.
1. Submit a pull request to the main branch of this repository.

