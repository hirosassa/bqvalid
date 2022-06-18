# bqvalid

## What bqvalid does

`bqvalid` is the SQL validator tool for BigQuery standard SQL.
`bqvalid` fails with error message if there's the expression that will cause full scan, print as it is otherwise.

## Usage

```shell
cat sample.sql | bqvalid
```

If the SQL is invalid, `bqvalid` outputs the reason and its position like:
```
Full scan will cause! Compared _TABLE_SUFFIX with subquery
start at: line 5
end at: line 7
expression:
(
  select dt from dates
)
```
