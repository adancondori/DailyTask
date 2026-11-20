# Validar las métricas de una instalación

Este archivo describe el procedimiento; no contiene resultados de producción ni acredita una conexión real.

1. Ejecuta cada SQL/NRQL guardado, sin modificaciones del modelo, con una ventana acotada y permisos de lectura. Registra fecha, ventana, estado, filas y cualquier truncamiento.
2. Para pagos en ClickHouse utiliza `payments FINAL`. Comprueba el coste de lectura y que los JOIN no multipliquen registros.
3. Contrasta volumen y conteos con una consulta independiente para un gateway y moneda conocidos. Conserva importes decimales y no sumes divisas distintas.
4. Contrasta las renovaciones con un total independiente sin JOIN. Mantén separados estados pendientes y atribuciones desconocidas.
5. Trata las autorizaciones sin capturar como un saldo puntual; compara mediciones archivadas y no sumes saldos diarios.
6. Calcula p95 sobre los eventos de cada periodo, sin promediar percentiles diarios. Registra cobertura y evita comparar ventanas incompletas.
7. Solo informa una tasa de aprobación o reembolsos fallidos si existe una fuente completa y validada. Cuando falte, indica No disponible y su causa.
8. Conserva resultados y cotejos en `consultas/evidencia-*.json` y el registro de validación en `consultas/VALIDACION.md`; ambos quedan fuera de Git. Los reportes y las series diarias se guardan en `salida/`, también ignorada.

Las pruebas de fixtures validan comportamiento con datos sintéticos. Las lecturas reales y la conciliación de cifras requieren evidencia propia de cada instalación.
