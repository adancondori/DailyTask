# El Diario — plan

Periódico personal diario en HTML y audio, generado cada mañana en la Mac.
Referencia de estructura: https://github.com/yesidays/diario-agente (Python + Gemini).
Este proyecto se escribe en **Rust**.

Estado: plan aprobado el 2026-11-19. Nada implementado todavía; no se empieza a
programar hasta que yo lo pida.

## 1. Qué debe traer cada edición

| Sección | Contenido | Fuente |
|---|---|---|
| Top 3 del día | Lo más urgente de todas las secciones | Redacción del modelo |
| Hecho ayer | Día hábil anterior. Primero lo mío: PRs abiertos, mergeados y revisados, commits, tickets que cambiaron de estado, reuniones. Después lo del equipo, por persona | `gh` + Jira + Calendar |
| Agenda | Eventos de hoy y mañana, choques, cancelaciones | Google Calendar |
| Correo | Personas primero; máquinas agrupadas por tipo; promociones solo contadas | Gmail |
| Producción | Errores de Honeybadger agrupados por tipo, pagos destacados | Gmail (alertas Honeybadger) |
| Mis PRs | PRs abiertos míos: CI, reviews, conflictos, días sin movimiento | `gh` |
| Esperan mi review | PRs donde me pidieron review | `gh` |
| Equipo | Todos los tickets del tablero CORE 590, de todo el equipo, agrupados por persona: estado + PR enlazado + días sin movimiento. Radhames y Santiago primero | Jira + `gh` |
| Alertas de equipo | PR sin review > 2 días, ticket In Progress sin commits > 3 días, ticket sin PR | Calculado en Rust |
| Slack | Menciones y DMs sin responder, hilos donde me esperan; canales de bots solo contados | Slack |
| Clima | Santa Cruz de la Sierra: temperatura, máxima, mínima, lluvia | Open-Meteo (sin API key) |
| Dólar | Bolivia, dos valores: oficial (BCB) y paralelo (USDT/BOB en Binance P2P o similar), con la brecha entre ambos y el cambio diario y a 5 días del paralelo | BCB + Binance P2P; fuente exacta por verificar en el paso 3 |
| Noticias | 5–7 notas con fuentes sobre: fintech en LATAM, Adyen, IA para desarrollo | Búsqueda web |
| Audio | Guion de ~2 minutos leído en voz alta | `say` de macOS |
| Enlace al reporte de equipo | Link a la página "Payments Impact" (sección 2b) | — |

"Hecho ayer" usa el día hábil anterior (el lunes muestra el viernes). La tabla de
Equipo lleva la misma columna por persona: qué cerró cada uno ayer.

Reglas de ruido aprendidas de la revisión real del 2026-11-19:

- Honeybadger: agrupar por clase de error y contar; nunca listar correo por correo.
- Slack: excluir la cola de `#deployments` de las menciones (unas 20 al día).
- Canales de bots (`#dev-notifications`, `#alertas-infra-devops`, `#rack-attack-alerts`,
  `#payment_intents_cancellation_tracker`): una línea de conteo, salvo alertas sin resolver.

## 2. Arquitectura

Un binario Rust, `diario`, con cuatro etapas. Cada etapa escribe un archivo, así
se puede repetir una sin rehacer las anteriores.

```
recolectar ──► datos.json ──► editar ──► edicion.json ──► render ──► AAAA-MM-DD.html
                                              │                         ultimo.html
                                              └──► guion ──► say ──► AAAA-MM-DD.m4a
```

1. **recolectar**: junta los datos crudos en `datos.json`.
2. **editar**: el modelo redacta la edición contra un esquema fijo (`edicion.json`).
3. **render**: plantilla HTML. Los números salen de `datos.json`, no del modelo.
4. **audio**: `say -v Paulina` sobre el guion.

### De dónde saca los datos el binario

| Fuente | Cómo | Credenciales nuevas |
|---|---|---|
| GitHub | subproceso `gh ... --json` | Ninguna (ya autenticado) |
| Clima, dólar | HTTP directo desde Rust | Ninguna |
| Gmail, Calendar, Slack, Jira, noticias | subproceso `claude -p` con herramientas de solo lectura, salida JSON | Ninguna (usa las conexiones existentes) |
| Redacción de la edición | subproceso `claude -p` con esquema JSON | Ninguna |

Los recolectores van detrás de un contrato JSON por fuente. Si después se quiere
un cliente nativo en Rust para Jira o Slack (con su propio token), se cambia ese
recolector sin tocar el resto.

Alternativa descartada por ahora: clientes nativos en Rust para todo. Exige OAuth
de Google, token de Slack, token de Jira y una API key de modelo, más el código
de cada cliente.

## 2b. Reporte del equipo Payments (página aparte)

Reporte nuevo y propio, dentro de este proyecto, pensado para responderle al jefe
cuando pregunta por el equipo. No actualiza el dashboard "Payments Impact"
existente (https://claude.ai/artifact/GZRhfWgX8tspJMuU6u2iNU); ese queda solo como
referencia de qué preguntas responde.

Salida: un artifact de claude.ai (privado por defecto, se comparte por link con el
jefe), enlazado desde el diario. La corrida genera los datos en
`salida/equipo/` y los publica siempre a la misma URL. Vistas por semana, quincena
y mes. Alcance: todos los gateways y todo el equipo del tablero CORE 590.

### Secciones

| Sección | Qué responde | Fuente |
|---|---|---|
| Resumen | 3 a 5 líneas: qué se entregó, qué mejoró, qué preocupa | Redacción del modelo sobre los datos de abajo |
| Entrega por persona | Cada miembro del tablero CORE 590: tickets cerrados y en curso, PRs mergeados, primer deploy a producción, días de ciclo | Jira + `gh` + cola de `#deployments` |
| Sprint | Comprometido vs. terminado, lo que se arrastra | Jira |
| Salud de pagos | Por cada gateway activo (todos): volumen, tasa de aprobación, autorizados sin capturar, renovaciones no cobradas, reembolsos fallidos | ClickHouse (agregados), réplica MySQL (detalle puntual) |
| Errores de pagos | Familias de error: nuevas, abiertas, cerradas, avisos por día y el PR que las cerró | Honeybadger |
| Rendimiento | Latencia p95 y tasa de error de las transacciones de pago, jobs de captura y renovación, webhooks de gateways | New Relic |
| Riesgos abiertos | Severidad, cifra, evidencia y siguiente paso | Curado a mano (`equipo/riesgos.toml`) |
| Iniciativas | Qué llegó a producción y su impacto | Curado a mano (`equipo/iniciativas.toml`) |

### Cómo se calculan los números

- Cada métrica es una consulta guardada en el repo (`consultas/*.sql` para
  ClickHouse y MySQL, `consultas/*.nrql` para New Relic). El binario las ejecuta
  tal cual; el modelo no escribe consultas en la corrida diaria.
- Agregados sobre pagos van a ClickHouse (`payments FINAL`), no a la réplica: la
  tabla `payments` tiene unos 52 millones de filas. La réplica solo para consultas
  acotadas por id o por fecha corta.
- Todas las consultas son de lectura y llevan `LIMIT` y ventana de fechas.
- Los resultados se guardan por día en `salida/equipo/AAAA-MM-DD/*.json`, así las
  series históricas salen de los archivos y no de reconsultar.
- Riesgos e iniciativas no se generan: los edito a mano. El binario deja
  sugerencias en `salida/equipo/propuestas.md`.

Acceso a las fuentes: igual que el resto del proyecto, por `claude -p` con las
conexiones existentes (ClickHouse, réplica MySQL, New Relic, Honeybadger) y solo
herramientas de lectura. Cada una se puede cambiar por un cliente nativo en Rust
más adelante.

Etapa nueva en el binario: `diario equipo`. Corre después de `recolectar` y
reutiliza los datos de GitHub, Jira y Honeybadger ya juntados.

Publicación: siempre manual. La corrida de la mañana refresca los datos locales
del reporte pero no publica. Se publica con el botón "Sync reporte" de la página
local o con `diario equipo --publicar`, que actualiza el artifact en la misma URL.
Es la única escritura hacia afuera del proyecto. Los pasos quedan documentados en
`README.md` (crear el artifact la primera vez, revisar, publicar, compartir el link,
qué hacer si falla).

El botón vive en la página local, no dentro del artifact: una página de claude.ai
no puede lanzar un programa en mi Mac. El artifact muestra la fecha y hora de su
última actualización.

## 2c. Botones de sync y servidor local

Un HTML abierto como archivo no puede ejecutar el binario, así que `diario servir`
levanta un servidor mínimo en `127.0.0.1` (solo esta máquina) que sirve `salida/`
y expone dos acciones:

| Botón | Qué hace |
|---|---|
| **Sync** (en el diario) | Corre todo de nuevo: recolectar, editar, render y audio. Muestra el avance por fuente y recarga la página al terminar |
| **Sync reporte** (en el diario, junto al enlace del reporte) | Refresca los datos del equipo y publica el artifact |

- Una sola corrida a la vez; el botón queda deshabilitado mientras corre.
- Si una fuente falla, el avance lo dice y el resto sigue.
- `launchd` mantiene el servidor arriba y lanza la corrida de las 7:00.

## 2d. Diseño y UX

Objetivo: que se entienda de un vistazo qué requiere acción y qué es solo contexto.

**Sistema de diseño**

- Diario (página local): **Pico CSS v2**, un solo archivo CSS copiado al repo. HTML
  semántico sin clases, modo claro y oscuro automático, sin paso de build y
  funciona sin red. Encima, un archivo chico de tokens propios para estados.
- Reporte (artifact): el tipo **Dashboard** de claude.ai, que trae su propio
  sistema de diseño, gráficos y tablas. Es el mismo que usa "Payments Impact", así
  el jefe ve algo familiar.

**Reglas que valen para las dos páginas**

- Mismo vocabulario de estado en todo: `Requiere acción`, `En curso`, `Bloqueado`,
  `Listo`, `Informativo`. Siempre color + ícono + texto, nunca solo color.
- Orden fijo del diario: Top 3 → Hecho ayer → Hoy (agenda, correo, Slack) →
  Trabajo (PRs, equipo, producción) → Contexto (clima, dólar, noticias).
- Cada elemento accionable tiene un solo enlace que lleva al PR, ticket, correo o
  hilo. Nada de texto sin destino.
- El ruido se cuenta, no se lista: "23 alertas de Honeybadger, 5 de pagos".
  El detalle queda plegado.
- Los números van con su comparación: contra ayer en el diario, contra el periodo
  anterior en el reporte.
- Cada sección muestra cuándo se actualizó y, si falló, lo dice ahí mismo en vez
  de quedar vacía.
- Tablas del equipo: una fila por ticket, columnas fijas (persona, ticket, estado,
  PR, días sin movimiento), ordenadas por lo más atascado primero.
- Se lee bien en el teléfono: una columna, tablas que pasan a tarjetas.
- Texto en español; nombres técnicos, tickets y gateways tal cual.

### Seguridad

- Las corridas de `claude -p` llevan una lista cerrada de herramientas de solo
  lectura. No pueden enviar correo, publicar en Slack, comentar ni mover tickets.
- El contenido de correos y mensajes es dato no confiable; el prompt lo declara y
  los mensajes que piden algo a una IA se marcan como sospechosos.
- Sin secretos en el repo. `config.toml` no lleva tokens.

### Fallos

- Una fuente que falla deja su sección con "no disponible" y el diario sale igual.
- Si falla la redacción, no se publica una edición vacía; queda `ultimo.html` de ayer.
- Si falla el audio, sale el HTML sin audio.

## 3. Estructura del proyecto

```
~/Projects/diario/
  Cargo.toml
  config.toml            # ciudad, moneda, temas, equipo, canales, horarios
  src/
    main.rs              # CLI: diario [recolectar|editar|render|audio|equipo|servir|todo]
    servir.rs            # servidor local y botones de sync
    equipo.rs            # reporte del equipo
    config.rs
    recolectar/          # un archivo por fuente: github, clima, dolar, claude
    modelo.rs            # tipos de datos.json y edicion.json (serde)
    alertas.rs           # reglas de equipo
    editar.rs
    render.rs
    audio.rs
  plantilla.html
  estilos/               # pico.min.css + tokens.css
  consultas/             # *.sql y *.nrql del reporte
  equipo/                # riesgos.toml, iniciativas.toml (curados a mano)
  salida/                # ediciones, audio, datos.json y edicion.json por día
  launchd/com.diario.plist
  README.md              # uso diario y pasos de publicación del reporte
```

Crates: `serde`, `serde_json`, `toml`, `ureq`, `minijinja`, `clap`, `anyhow`,
`chrono`, `tiny_http`.

## 4. Pasos de construcción

Cada paso deja algo que se puede ejecutar y revisar.

1. **Verificación previa**: confirmar que `claude -p` en modo headless ve las
   conexiones de Gmail, Slack y Jira y devuelve JSON. Conectar Google Calendar
   (hoy está sin autenticar). Si headless no ve las conexiones, este plan cambia
   a clientes nativos para esas fuentes.
2. **Esqueleto**: `cargo new`, `config.toml`, CLI, tipos de `datos.json`.
3. **Fuentes directas**: clima, dólar, `gh` (mis PRs, review pendiente, PRs del equipo).
4. **Render mínimo**: plantilla HTML con esas tres secciones, ya con Pico CSS, los
   tokens de estado y el orden fijo de la sección 2d. Primer diario visible.
5. **Fuentes vía Claude**: Gmail, Calendar, Slack, Jira, con las reglas de ruido.
6. **Alertas de equipo**: cruce Jira ↔ PR y sus reglas, con pruebas.
7. **Edición**: Top 3, resúmenes y guion contra el esquema.
8. **Noticias**: búsqueda web con fuentes numeradas.
9. **Audio**: `say` y reproductor en el HTML.
10. **Servidor y programación**: `diario servir` con el botón Sync y su avance;
    `launchd` lo mantiene arriba y lanza la corrida a las 7:00 (GMT-4), lunes a
    viernes.
11. **Hecho ayer**: sección propia y columna por persona en Equipo (sale de los
    datos de los pasos 3 y 5; se construye junto con el paso 6).
12. **Reporte de equipo**, en este orden:
    a. Entrega por persona y sprint (datos que ya existen del paso 6).
    b. Errores de pagos desde Honeybadger.
    c. Salud de pagos: escribir y validar las consultas de ClickHouse una por una
       contra cifras conocidas antes de guardarlas.
    d. Rendimiento desde New Relic.
    e. Archivos curados y `propuestas.md`.
    f. Crear el artifact desde el tipo Dashboard, revisarlo juntos y recién
       entonces compartir el link.
    g. Botón "Sync reporte" y `README.md` con los pasos de publicación.

Pruebas: unitarias para `alertas.rs`, agrupación de Honeybadger y filtros de
Slack, con datos de ejemplo guardados en `fixtures/`. Un modo `--demo` que
renderiza desde fixtures sin red.

## 5. Para después (no entra en la primera versión)

- Qué cambió desde ayer (comparar `datos.json` de dos días).
- Edición semanal los viernes para los 1:1 con Radhames y Santiago.
- Entrega del audio y el Top 3 por Telegram (bot Jarvis).
- Borradores de respuesta para correo y Slack.
- Preparación de reuniones: tickets y PRs relacionados con cada evento.
- Pendientes de las reuniones de ayer (notas de Gemini).
- Voz TTS de mejor calidad si `say` no convence.

## 6. Decisiones abiertas

Decidido:

- Ciudad: Santa Cruz de la Sierra, Bolivia (`America/La_Paz`).
- Dólar: oficial de Bolivia y paralelo (Binance P2P o similar).
- Jira: proyecto CORE, tablero 590
  (https://paybycourt.atlassian.net/jira/software/projects/CORE/boards/590),
  todos los tickets de todo el equipo.
- Reporte de equipo: nuevo, propio, publicado como artifact, con todos los gateways.

- Noticias: fintech en LATAM, Adyen, IA para desarrollo.
- Hora: 7:00 GMT-4, lunes a viernes, más botón Sync para correr a demanda.
- Publicación del reporte: manual, con botón "Sync reporte" y pasos documentados.
- "Hecho ayer": primero lo mío, después el equipo.
- Diseño: Pico CSS para el diario, tipo Dashboard para el artifact.

Se resuelve al construir (solo se pregunta si algo no cruza):

1. Usuarios de GitHub de cada miembro del tablero, cruzando Jira con GitHub.
2. Transacciones y jobs de New Relic para "Rendimiento".
3. Fuente del dólar paralelo que responda sin API key.
4. Si `claude -p` sin sesión interactiva puede publicar el artifact; si no, el
   botón "Sync reporte" refresca los datos y la publicación se hace desde una
   sesión de Claude Code con un comando documentado.
8. "Hecho ayer": ¿solo lo mío, o también un resumen del equipo arriba?
