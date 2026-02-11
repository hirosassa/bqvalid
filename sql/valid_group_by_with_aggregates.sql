-- All non-aggregated columns are in GROUP BY
SELECT col1, COUNT(col2) as cnt, SUM(col3) as total
FROM my_table
GROUP BY col1;

-- Multiple columns in GROUP BY
SELECT col1, col2, MAX(col3) as max_val
FROM my_table
GROUP BY col1, col2;

-- Query without GROUP BY (no violation)
SELECT col1, col2, col3
FROM my_table;
