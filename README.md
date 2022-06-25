# bqvalid

## What bqvalid does

`bqvalid` is the SQL validator tool for BigQuery standard SQL.
`bqvalid` fails with error message if there's the expression that will cause full scan, print as it is otherwise.

## Usage

```shell
cat sample.sql | bqvalid
```

If the SQL is contained the expressions that comparing ``_TABLE_SUFFIX` with subquery, `bqvalid` outputs the reason and its position like:
```
5:7: Full scan will cause! Should not compare _TABLE_SUFFIX with subquery
```

Also, you can input file paths, too:

```
bqvalid one.sql two.sql three.sql
```

Then, the output will as follows:
```
one.sql:6:6: Full scan will cause! Should not compare _TABLE_SUFFIX with subquery
three.sql:5:19: Full scan will cause! Should not compare _TABLE_SUFFIX with subquery
```
