SELECT toDate(created_at, 'America/La_Paz') AS fecha,
    gateway, currency, count() AS registros,
    countIf(status IN ('authorized','captured','partially_refunded','totally_refunded','refunded_to_credit')) AS aprobados_persistidos,
    countIf(status='authorized') AS autorizados_sin_capturar,
    countIf(status='captured') AS capturados,
    sumIf(reservation_paid_out,status='captured') AS volumen_capturado
FROM pbp_productionDB_optimized.payments FINAL
WHERE created_at >= toDateTime('{{inicio}}','UTC')
  AND created_at < toDateTime('{{fin}}','UTC')
  AND _peerdb_is_deleted=0 AND gateway!=''
GROUP BY fecha,gateway,currency ORDER BY fecha,gateway,currency LIMIT 10000
