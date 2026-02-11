-- Test all BigQuery aggregate functions
SELECT
  col1,
  -- Standard aggregate functions
  ANY_VALUE(col2) as any_val,
  ARRAY_AGG(col3) as arr,
  ARRAY_CONCAT_AGG(col4) as arr_concat,
  AVG(col5) as avg_val,
  BIT_AND(col6) as bit_and_val,
  BIT_OR(col7) as bit_or_val,
  BIT_XOR(col8) as bit_xor_val,
  COUNT(col9) as cnt,
  COUNTIF(col10 > 0) as cnt_if,
  LOGICAL_AND(col11) as logical_and_val,
  LOGICAL_OR(col12) as logical_or_val,
  MAX(col13) as max_val,
  MAX_BY(col14, col15) as max_by_val,
  MIN(col16) as min_val,
  MIN_BY(col17, col18) as min_by_val,
  STRING_AGG(col19) as str_agg,
  SUM(col20) as sum_val,
  -- Approximate aggregate functions
  APPROX_COUNT_DISTINCT(col21) as approx_cnt,
  APPROX_QUANTILES(col22, 4) as approx_quant,
  APPROX_TOP_COUNT(col23, 10) as approx_top_cnt,
  APPROX_TOP_SUM(col24, col25, 10) as approx_top_sum,
  -- Statistical aggregate functions
  CORR(col26, col27) as corr_val,
  COVAR_POP(col28, col29) as covar_pop_val,
  COVAR_SAMP(col30, col31) as covar_samp_val,
  STDDEV(col32) as stddev_val,
  STDDEV_POP(col33) as stddev_pop_val,
  STDDEV_SAMP(col34) as stddev_samp_val,
  VAR_POP(col35) as var_pop_val,
  VAR_SAMP(col36) as var_samp_val,
  VARIANCE(col37) as variance_val
FROM my_table
GROUP BY col1;
