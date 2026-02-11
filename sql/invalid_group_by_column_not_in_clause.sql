-- col2 is not in GROUP BY and not in an aggregate function
SELECT col1, col2, COUNT(*) as cnt
FROM my_table
GROUP BY col1;
