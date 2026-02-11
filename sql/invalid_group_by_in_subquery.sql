-- Invalid GROUP BY in subquery should be detected
SELECT *
FROM (
  SELECT col1, col2, COUNT(*) as cnt
  FROM my_table
  GROUP BY col1
) sub;
