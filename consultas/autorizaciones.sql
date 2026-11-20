SELECT
    if(gateway='', 'sin_gateway_identificado', gateway) AS gateway,
    currency,
    count() AS autorizados_sin_capturar,
    countIf(created_at < now() - INTERVAL 7 DAY) AS mayores_siete_dias
FROM pbp_productionDB_optimized.payments FINAL
WHERE _peerdb_is_deleted = 0
  AND status = 'authorized'
  AND created_at >= toDateTime('1970-01-01 00:00:00', 'UTC')
  AND created_at < toDateTime('{{fin}}', 'UTC')
GROUP BY gateway, currency
ORDER BY gateway, currency
LIMIT 1000
