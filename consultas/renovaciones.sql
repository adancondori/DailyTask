WITH renovaciones AS (
 SELECT id, payment_id, user_token_id, state FROM pbp_base_prod.membership_payments FINAL
 WHERE _peerdb_is_deleted=0 AND origin='automatic_renewal' AND state!='pending'
 AND created_at >= toDateTime('{{inicio}}','UTC') AND created_at < toDateTime('{{fin}}','UTC')
), pagos AS (
 SELECT id, gateway FROM pbp_base_prod.payments FINAL
 WHERE _peerdb_is_deleted=0 AND id IN (SELECT payment_id FROM renovaciones WHERE payment_id>0)
), tokens AS (
 SELECT id, gateway FROM pbp_base_prod.user_tokens FINAL
 WHERE _peerdb_is_deleted=0 AND id IN (SELECT user_token_id FROM renovaciones WHERE user_token_id>0)
)
SELECT if(p.id>0 AND p.gateway!='',p.gateway,if(t.id=0,'sin_gateway_identificado', multiIf(t.gateway=0,'card_connect',t.gateway=1,'pixel_pay',t.gateway=2,'azul_pay',t.gateway=3,'kushki_pay',t.gateway=4,'luka_pay',t.gateway=5,'stripe',t.gateway=6,'icount',t.gateway=7,'dpo_pay',t.gateway=8,'mitec',t.gateway=9,'pay_fast',t.gateway=10,'bac',t.gateway=11,'one_pay',t.gateway=12,'pay_code',t.gateway=13,'razor_pay',t.gateway=14,'xendit',t.gateway=15,'adyen',concat('gateway_',toString(t.gateway))))) AS gateway,
count() AS renovaciones,
countIf(r.state IN ('payment_failed','no_default_payment_method','renewal_charge_unconfirmed')) AS no_cobradas,
countIf(r.state='payment_failed') AS rechazadas,
countIf(r.state='no_default_payment_method') AS sin_metodo,
countIf(r.state='renewal_charge_unconfirmed') AS sin_confirmar,
countIf(r.state='processing') AS procesando
FROM renovaciones r LEFT ANY JOIN pagos p ON r.payment_id=p.id LEFT ANY JOIN tokens t ON r.user_token_id=t.id
GROUP BY gateway ORDER BY gateway LIMIT 100
