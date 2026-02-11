-- Qualified column name not in GROUP BY
SELECT t.col1, t.col2, COUNT(*) as cnt
FROM my_table t
GROUP BY t.col1;
