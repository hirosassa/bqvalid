-- Multiple columns not in GROUP BY
SELECT col1, col2, col3, COUNT(*) as cnt
FROM my_table
GROUP BY col1;
