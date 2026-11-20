SELECT id, gateway, currency, status, created_at, captured_at
FROM payments
WHERE id = {{id}}
  AND created_at >= '{{inicio}}'
  AND created_at < '{{fin}}'
LIMIT 1
