-- Mixed case aggregate functions should work
SELECT col1, Count(col2) as cnt, SuM(col3) as total, mAx(col4) as max_val
FROM my_table
GROUP BY col1;
