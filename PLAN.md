# El Diario — plan

Periódico personal diario en HTML y audio, generado cada mañana en la Mac.
Referencia de estructura: https://github.com/yesidays/diario-agente (Python + Gemini).
Este proyecto se escribe en **Rust**.

Estado: borrador, pendiente de aprobación. Nada implementado todavía.

## 1. Qué debe traer cada edición

| Sección | Contenido | Fuente |
|---|---|---|
| Top 3 del día | Lo más urgente de todas las secciones | Redacción del modelo |
| Agenda | Eventos de hoy y mañana, choques, cancelaciones | Google Calendar |
| Correo | Personas primero; máquinas agrupadas por tipo; promociones solo contadas | Gmail |
| Producción | Errores de Honeybadger agrupados por tipo, pagos destacados | Gmail (alertas Honeybadger) |
| Mis PRs | PRs abiertos míos: CI, reviews, conflictos, días sin movimiento | `gh` |
| Esperan mi review | PRs donde me pidieron review | `gh` |
| Equipo | Radhames y Santiago: tickets Jira del sprint + PR enlazado + estado + días sin movimiento | Jira + `gh` |
| Alertas de equipo | PR sin review > 2 días, ticket In Progress sin commits > 3 días, ticket sin PR | Calculado en Rust |
| Slack | Menciones y DMs sin responder, hilos donde me esperan; canales de bots solo contados | Slack |
| Clima | Temperatura, máxima, mínima, lluvia | Open-Meteo (sin API key) |
| Dólar | Valor, cambio diario y a 5 días | open.er-api.com / Frankfurter (sin API key) |
| Noticias | 5–7 notas de mis temas con fuentes | Búsqueda web |
| Audio | Guion de ~2 minutos leído en voz alta | `say` de macOS |

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
    main.rs              # CLI: diario [recolectar|editar|render|audio|todo]
    config.rs
    recolectar/          # un archivo por fuente: github, clima, dolar, claude
    modelo.rs            # tipos de datos.json y edicion.json (serde)
    alertas.rs           # reglas de equipo
    editar.rs
    render.rs
    audio.rs
  plantilla.html
  salida/                # ediciones, audio, datos.json y edicion.json por día
  launchd/com.diario.plist
```

Crates: `serde`, `serde_json`, `toml`, `ureq`, `minijinja`, `clap`, `anyhow`, `chrono`.

## 4. Pasos de construcción

Cada paso deja algo que se puede ejecutar y revisar.

1. **Verificación previa**: confirmar que `claude -p` en modo headless ve las
   conexiones de Gmail, Slack y Jira y devuelve JSON. Conectar Google Calendar
   (hoy está sin autenticar). Si headless no ve las conexiones, este plan cambia
   a clientes nativos para esas fuentes.
2. **Esqueleto**: `cargo new`, `config.toml`, CLI, tipos de `datos.json`.
3. **Fuentes directas**: clima, dólar, `gh` (mis PRs, review pendiente, PRs del equipo).
4. **Render mínimo**: plantilla HTML con esas tres secciones. Primer diario visible.
5. **Fuentes vía Claude**: Gmail, Calendar, Slack, Jira, con las reglas de ruido.
6. **Alertas de equipo**: cruce Jira ↔ PR y sus reglas, con pruebas.
7. **Edición**: Top 3, resúmenes y guion contra el esquema.
8. **Noticias**: búsqueda web con fuentes numeradas.
9. **Audio**: `say` y reproductor en el HTML.
10. **Programación**: `launchd`, lunes a viernes, y abrir el HTML al terminar.

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

1. Ciudad para el clima (zona horaria observada: GMT-4).
2. Moneda contra el dólar.
3. Temas de noticias.
4. Hora de la corrida (propuesta: 7:00, lunes a viernes).
5. Proyecto y tablero de Jira del equipo, y usuarios de GitHub de Radhames y Santiago.
