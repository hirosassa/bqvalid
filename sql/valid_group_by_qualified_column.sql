-- Qualified column names correctly used with GROUP BY
SELECT t.col1, t.col2, COUNT(t.col3) as cnt
FROM my_table t
GROUP BY t.col1, t.col2;

-- Join with qualified columns
SELECT t1.user_id, t2.category, COUNT(*) as cnt
FROM users t1
JOIN orders t2 ON t1.id = t2.user_id
GROUP BY t1.user_id, t2.category;
