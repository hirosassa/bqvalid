-- Mix of qualified and non-qualified columns
SELECT t1.col1, col2, t1.col3, COUNT(*) as cnt
FROM my_table t1
GROUP BY t1.col1, col2;
-- t1.col3 is not in GROUP BY
