WITH configurados AS (
SELECT gateway,countDistinct(facility) AS instalaciones_configuradas FROM (
 SELECT toString(f.uuid) AS facility,if(r.id=0 OR NOT r.active,'card_connect',r.default_value) AS gateway
 FROM (SELECT uuid FROM pbp_base_prod.facilities FINAL WHERE _peerdb_is_deleted=0 AND deleted_at=toDateTime(0) AND created_at<toDateTime('{{fin}}','UTC')) f
 LEFT ANY JOIN (SELECT id,owner,active,default_value FROM pbp_base_prod.rules FINAL WHERE _peerdb_is_deleted=0 AND name='default_payment_gateway' AND namespace='payments_rules' AND created_at<toDateTime('{{fin}}','UTC')) r ON toString(f.uuid)=r.owner
 UNION ALL
 SELECT r.owner AS facility,v.value AS gateway FROM
 (SELECT rule_id,value FROM pbp_base_prod.rule_values FINAL WHERE _peerdb_is_deleted=0 AND created_at<toDateTime('{{fin}}','UTC')) v
 INNER ANY JOIN (SELECT id,owner FROM pbp_base_prod.rules FINAL WHERE _peerdb_is_deleted=0 AND name='default_payment_gateway' AND namespace='payments_rules' AND active AND created_at<toDateTime('{{fin}}','UTC')) r ON v.rule_id=r.id
 WHERE r.owner IN (SELECT toString(uuid) FROM pbp_base_prod.facilities FINAL WHERE _peerdb_is_deleted=0 AND deleted_at=toDateTime(0) AND created_at<toDateTime('{{fin}}','UTC'))
) WHERE gateway IN ('card_connect','pixel_pay','azul_pay','kushki_pay','luka_pay','stripe','icount','dpo_pay','mitec','pay_fast','bac','one_pay','pay_code','razor_pay','xendit','adyen') GROUP BY gateway
), actividad AS (
SELECT gateway,currency,count() AS registros,
countIf(status IN ('authorized','captured','partially_refunded','totally_refunded','refunded_to_credit')) AS aprobados_persistidos,
countIf(status='authorized') AS autorizados_sin_capturar,
countIf(status='captured') AS capturados,
sumIf(reservation_paid_out,status='captured') AS volumen_capturado
FROM pbp_productionDB_optimized.payments FINAL
WHERE created_at>=toDateTime('{{inicio}}','UTC') AND created_at<toDateTime('{{fin}}','UTC') AND _peerdb_is_deleted=0 AND gateway!=''
GROUP BY gateway,currency
)
SELECT if(a.gateway='',c.gateway,a.gateway) AS gateway,
if(a.gateway='','Sin actividad',a.currency) AS currency,
a.registros,a.aprobados_persistidos,a.autorizados_sin_capturar,a.capturados,a.volumen_capturado,c.instalaciones_configuradas
FROM actividad a FULL OUTER JOIN configurados c ON a.gateway=c.gateway
ORDER BY gateway,currency LIMIT 1000
