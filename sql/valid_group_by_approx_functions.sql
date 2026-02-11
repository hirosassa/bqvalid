-- Test approximate aggregate functions specifically
SELECT
  user_id,
  APPROX_COUNT_DISTINCT(product_id) as unique_products,
  APPROX_QUANTILES(price, 4) as price_quartiles,
  APPROX_TOP_COUNT(category, 5) as top_categories,
  APPROX_TOP_SUM(amount, item_name, 10) as top_items_by_amount
FROM sales_table
GROUP BY user_id;
