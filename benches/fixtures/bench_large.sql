-- Large benchmark: 20 CTEs with high complexity
WITH raw_users AS (
  SELECT
    id, name, email, age, country, city, postal_code,
    created_at, updated_at, last_login_at, status,
    unused1, unused2, unused3
  FROM `project.dataset.users`
),
raw_orders AS (
  SELECT
    order_id, user_id, product_id, quantity, amount,
    discount, tax, shipping_cost, order_date, status,
    unused1, unused2, unused3
  FROM `project.dataset.orders`
),
raw_products AS (
  SELECT
    product_id, name, category, subcategory, brand,
    price, cost, stock_quantity, supplier_id,
    unused1, unused2, unused3
  FROM `project.dataset.products`
),
user_demographics AS (
  SELECT
    id, name, email, age, country, city,
    CASE WHEN age < 25 THEN 'young'
         WHEN age < 45 THEN 'middle'
         ELSE 'senior' END as age_group,
    unused_demo1, unused_demo2
  FROM raw_users
  WHERE status = 'active'
),
order_details AS (
  SELECT
    o.order_id, o.user_id, o.product_id,
    o.quantity, o.amount, o.discount, o.tax,
    p.name as product_name, p.category, p.brand,
    unused_detail1, unused_detail2, unused_detail3
  FROM raw_orders o
  JOIN raw_products p ON o.product_id = p.product_id
),
user_order_summary AS (
  SELECT
    user_id,
    COUNT(DISTINCT order_id) as order_count,
    COUNT(DISTINCT product_id) as unique_products,
    SUM(amount) as total_amount,
    SUM(quantity) as total_quantity,
    AVG(amount) as avg_order_amount,
    MAX(amount) as max_order_amount,
    MIN(amount) as min_order_amount,
    unused_summary1, unused_summary2
  FROM order_details
  GROUP BY user_id
),
product_performance AS (
  SELECT
    product_id, product_name, category, brand,
    COUNT(order_id) as times_ordered,
    SUM(quantity) as total_sold,
    SUM(amount) as revenue,
    unused_perf1, unused_perf2, unused_perf3
  FROM order_details
  GROUP BY product_id, product_name, category, brand
),
category_stats AS (
  SELECT
    category,
    COUNT(DISTINCT product_id) as product_count,
    SUM(total_sold) as category_sales,
    AVG(revenue) as avg_revenue,
    unused_cat1, unused_cat2
  FROM product_performance
  GROUP BY category
),
user_segments AS (
  SELECT
    u.id, u.name, u.email, u.age_group,
    s.order_count, s.total_amount,
    CASE
      WHEN s.total_amount > 50000 THEN 'platinum'
      WHEN s.total_amount > 20000 THEN 'gold'
      WHEN s.total_amount > 5000 THEN 'silver'
      ELSE 'bronze'
    END as segment,
    unused_seg1, unused_seg2
  FROM user_demographics u
  JOIN user_order_summary s ON u.id = s.user_id
),
monthly_sales AS (
  SELECT
    DATE_TRUNC(order_date, MONTH) as month,
    user_id, product_id,
    COUNT(*) as monthly_orders,
    SUM(amount) as monthly_revenue,
    unused_monthly1, unused_monthly2
  FROM raw_orders
  GROUP BY month, user_id, product_id
),
user_product_affinity AS (
  SELECT
    user_id, product_id,
    COUNT(*) as purchase_count,
    AVG(quantity) as avg_quantity,
    unused_affinity1
  FROM order_details
  GROUP BY user_id, product_id
),
top_customers AS (
  SELECT
    id, name, segment, total_amount,
    RANK() OVER (ORDER BY total_amount DESC) as customer_rank,
    unused_top1, unused_top2
  FROM user_segments
  WHERE segment IN ('platinum', 'gold')
),
product_recommendations AS (
  SELECT
    a.user_id,
    a.product_id,
    a.purchase_count,
    p.category,
    p.brand,
    unused_rec1, unused_rec2
  FROM user_product_affinity a
  JOIN product_performance p ON a.product_id = p.product_id
  WHERE a.purchase_count > 2
),
user_lifecycle AS (
  SELECT
    id, name, created_at,
    DATE_DIFF(CURRENT_DATE(), created_at, DAY) as days_since_signup,
    CASE
      WHEN DATE_DIFF(CURRENT_DATE(), last_login_at, DAY) < 7 THEN 'active'
      WHEN DATE_DIFF(CURRENT_DATE(), last_login_at, DAY) < 30 THEN 'at_risk'
      ELSE 'churned'
    END as lifecycle_stage,
    unused_lifecycle1, unused_lifecycle2, unused_lifecycle3
  FROM raw_users
),
churn_analysis AS (
  SELECT
    u.id, u.lifecycle_stage,
    s.order_count, s.total_amount,
    EXTRACT(DAYOFWEEK FROM MAX(o.order_date)) as last_order_day,
    unused_churn1, unused_churn2
  FROM user_lifecycle u
  LEFT JOIN user_order_summary s ON u.id = s.user_id
  LEFT JOIN raw_orders o ON u.id = o.user_id
  GROUP BY u.id, u.lifecycle_stage, s.order_count, s.total_amount
),
cohort_analysis AS (
  SELECT
    DATE_TRUNC(created_at, MONTH) as cohort_month,
    COUNT(DISTINCT id) as cohort_size,
    AVG(total_amount) as avg_ltv,
    unused_cohort1, unused_cohort2
  FROM user_segments
  GROUP BY cohort_month
),
rfm_scores AS (
  SELECT
    user_id,
    DATE_DIFF(CURRENT_DATE(), MAX(order_date), DAY) as recency,
    COUNT(*) as frequency,
    SUM(amount) as monetary,
    unused_rfm1, unused_rfm2
  FROM raw_orders
  GROUP BY user_id
),
final_analytics AS (
  SELECT
    u.id, u.name, u.segment,
    r.recency, r.frequency, r.monetary,
    c.lifecycle_stage,
    RANK() OVER (PARTITION BY u.segment ORDER BY r.monetary DESC) as segment_rank,
    unused_final1, unused_final2, unused_final3
  FROM user_segments u
  JOIN rfm_scores r ON u.id = r.user_id
  JOIN user_lifecycle c ON u.id = c.id
),
aggregated_metrics AS (
  SELECT
    segment, lifecycle_stage,
    COUNT(*) as user_count,
    AVG(monetary) as avg_value,
    SUM(monetary) as total_value,
    unused_agg1
  FROM final_analytics
  GROUP BY segment, lifecycle_stage
)
SELECT
  segment,
  lifecycle_stage,
  user_count,
  avg_value,
  total_value
FROM aggregated_metrics
ORDER BY segment, lifecycle_stage
