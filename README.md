# El Diario

Periódico personal en Rust, HTML y audio local. Implementa el alcance de [PLAN.md](PLAN.md). Las cuatro conexiones reales están verificadas. El Dashboard privado está creado; la revisión conjunta y compartirlo siguen pendientes. Calendar conserva eventos y reuniones, pero su herramienta no permite consultar cancelaciones explícitamente: esa limitación se muestra en la sección. El demo no representa datos reales.

## Verlo sin conexiones

```sh
cp -n config.example.toml config.toml
cargo run -- --demo todo
cargo run -- --demo servir
open http://127.0.0.1:7331/ultimo.html
```

El modo demo usa `fixtures/`, funciona sin red y escribe en `salida/demo/`. `--sin-audio` omite `say`. El audio usa la voz Paulina y `afconvert` de macOS; el demo dura aproximadamente dos minutos.

## Preparar las fuentes reales

Se necesitan Rust, `gh` autenticado, Claude Code autenticado con sus conexiones existentes y macOS para audio/launchd. `config.toml` es local y no se versiona. Copia `config.example.toml` y adapta sus valores de demostración. No guardes tokens en ese archivo. Comprueba `gh auth status`, `claude auth status` y `diario verificar`. No pegues en el repositorio la salida de `claude mcp list`: puede mostrar argumentos privados de servidores.

Abre [Personalizar → Conectores](https://claude.ai/customize/connectors) con la misma cuenta de Claude Code. Selecciona **Google Calendar → Conectar**, inicia sesión con tu Google del trabajo y completa la autorización. Repite con **Atlassian / Atlassian MCP**, usando la cuenta de Jira y el sitio `paybycourt.atlassian.net`. Si falta un conector, búscalo con **+**; en Team/Enterprise el propietario debe habilitarlo primero. Comprueba que ambos estén conectados y ejecuta `diario verificar`. [Guía oficial de conexión](https://support.claude.com/en/articles/11176164-use-connectors-to-extend-claude-s-capabilities).

Gmail y Slack también usan conexiones existentes. Jira usa el conector «Atlassian» autorizado a paybycourt, con paginación controlada por Rust, parámetros exactos y extracción de las respuestas originales; el conector «Atlassian MCP» separado puede tener otros sitios autorizados. El filtro 11059 de CORE 590 y los integrantes están verificados en su configuración de Jira. Los historiales se conservan por ticket y updated; si no cambian, se reutilizan. El tablero es CORE 590. Se reutilizan los MCP existentes de ClickHouse, Honeybadger y New Relic. Slack dispone de un adaptador GET que reutiliza la sesión del MCP local sin copiar sus credenciales; si esa configuración no existe, usa el recolector Claude. Solo permite métodos de lectura y respeta Retry-After. Si una herramienta cambia de nombre, confirma su carácter de lectura y actualiza la lista exacta en `src/recolectar/claude.rs`; nunca añadas comodines. El hook bloquea herramientas fuera de esa lista y exige que SQL/NRQL coincida con las consultas guardadas.

`diario verificar` realiza lecturas mínimas; el inventario de conectores por sí solo no demuestra acceso. Sus llamadas observadas se guardan sin cuerpos en `salida/.auditoria/`. El cliente de Claude usa esquema Draft 7 porque la versión instalada rechaza el identificador Draft 2020-12.

Revisa usuarios GitHub, organización, repositorios, ciudad y canales en `config.toml`. El mapa `[identidades]` contiene solo coincidencias Jira/GitHub verificadas. Si falta una identidad, el reporte la indica; no supone que dos nombres similares representan a la misma persona.

## Uso diario

```sh
cargo build --release
./target/release/diario todo
./target/release/diario servir
```

Abre siempre `http://127.0.0.1:7331/ultimo.html` para usar los botones. Abrir el HTML como archivo permite leerlo, pero no sincronizar. **Sync** corre las etapas y muestra el avance por fuente. Una sola corrida puede ejecutarse, incluyendo CLI, servidor y horario programado. Si otra corrida tiene el lock, la nueva se rechaza.

Cada etapa se puede repetir:

```sh
diario recolectar
diario recolectar --fuente github
diario editar
diario render
diario audio
diario equipo
diario equipo --solo-edicion
```

Usa el binario de `target/release/` o añádelo a tu PATH. `recolectar --fuente` repite únicamente github, clima, dolar, gmail, calendar, slack, jira o noticias sobre el datos.json existente. `--config /ruta/config.toml` permite otra configuración. `--fecha AAAA-MM-DD` selecciona archivos de otra edición para editar/render/audio/equipo; la recolección real solo obtiene hoy. El día hábil anterior omite sábados y domingos, no feriados.

Salidas:

- `salida/AAAA-MM-DD/datos.json`, `edicion.json`, `guion.txt`.
- `salida/AAAA-MM-DD.html`, `AAAA-MM-DD.m4a`, `ultimo.html`.
- `salida/equipo/AAAA-MM-DD/`: consultas, snapshots, reporte y `publicacion.json` compacto.
- `salida/equipo/ultimo.html`, `propuestas.md`, `publicar-prompt.txt`.

Si falla una fuente, su sección muestra el error y las demás siguen. Si falla editar, `ultimo.html` se conserva. Si falla el audio, el HTML permanece. El dólar oficial se lee de la tabla BCB con fecha; el paralelo es la mediana de diez ofertas comerciales de compra USDT/BOB en Binance P2P, con un mínimo de tres. No es una cotización de efectivo. Cambios diario/a cinco días requieren esos archivos históricos; si faltan, dice historia insuficiente.

## Reporte Payments y publicación manual

`diario equipo` reutiliza GitHub/Jira/Slack de `datos.json` y refresca métricas externas. `diario equipo --solo-edicion` repite únicamente resumen y render desde el reporte guardado, conservando todas las vistas y archivos de fuentes; sirve para recuperar una redacción fallida sin repetir consultas. Conserva snapshots locales: las series leen los archivos guardados. Las cohortes de pagos y renovaciones que faltan dentro de los últimos sesenta días se recuperan una vez con consultas fijas, incluidos fines de semana. Los archivos completos se conservan; las métricas del último día se refrescan al ejecutar equipo. Semana, quincena y mes son ventanas móviles de 7, 15 y 30 días completos, comparadas con la ventana inmediatamente anterior. Los importes se suman con aritmética decimal, separados por moneda. Las variaciones se habilitan solo con cobertura completa en ambos periodos; los totales parciales llevan su cobertura. El demo incluye una serie sintética de sesenta días. Un cero requiere una consulta válida; la falta de datos no equivale a cero.

Honeybadger usa un cliente nativo de solo lectura que reutiliza la credencial privada ya configurada en su MCP. Consulta las familias de pagos con actividad en los últimos sesenta días completos y guarda los conteos diarios exactos en `salida/equipo/honeybadger/`. No copia credenciales al proyecto. El estado abierto/resuelto es el de su última lectura, cuya fecha se muestra; si la API no expone fecha de resolución o PR de cierre, se muestran sin confirmar. No se considera cerrada una familia solo porque dejó de enviar avisos. La carga inicial puede necesitar varias corridas por el límite de 360 lecturas/hora; conserva los días completos y respeta el reinicio indicado por Honeybadger. Las corridas posteriores solo consultan días nuevos. Referencia: [límites de la API](https://docs.honeybadger.io/api/getting-started/), [conteos dentro de la ventana](https://docs.honeybadger.io/api/faults/).

Salud incluye gateways configurados aunque no tengan movimientos. Las renovaciones usan la tabla histórica `membership_payments`, con el gateway del pago/token enlazado; los registros sin atribución verificable conservan su grupo propio. Aprobación real y reembolsos fallidos se muestran como **No disponible**, con la causa, según lo acordado para la primera versión.

El saldo total de autorizaciones sin capturar es una medición puntual, separada de la cohorte de pagos creados en cada ventana. Incluye las autorizaciones antiguas y destaca las de más de siete días. Se compara con las mediciones archivadas de hace 7, 15 o 30 días; si todavía no existen, indica comparación no disponible. Nunca suma saldos diarios.

Edita `equipo/riesgos.toml` y `equipo/iniciativas.toml` a mano. La corrida solo propone cambios en `salida/equipo/propuestas.md`. Las consultas están en `consultas/`. El procedimiento de validación está en `consultas/VALIDACION.example.md`; guarda las evidencias de tu instalación en `consultas/VALIDACION.md` y `consultas/evidencia-*.json`, que se mantienen locales. No interpretes la proporción de pagos persistidos como tasa de aprobación del gateway: los rechazos antes de guardar Payment pueden faltar. No se mezclan monedas al sumar volumen.

La corrida de la mañana **no publica**. La única escritura externa prevista es la actualización manual del nuevo Dashboard privado. No se modifica el artifact Payments Impact de referencia.

Flujo de primera publicación:

1. Ejecuta `diario equipo` y revisa el HTML local y sus fuentes/limitaciones.
2. Abre una sesión interactiva de Claude Code en este directorio con `claude` y ejecuta `/diario-publicar`. El comando consume `salida/equipo/publicar-prompt.txt` y el `publicacion.json` compacto del día. Si la sesión no dispone de edición de Dashboard, abre la conversación existente del reporte en Claude / Playbypoint, adjunta ese `publicacion.json` y pega el prompt; esta vía se probó en el Dashboard privado.
3. Si ya hay un Dashboard en `reporte_url`, actualízalo conservando esa URL. En una instalación sin Dashboard, crea uno **nuevo del tipo Dashboard**, privado. En la interfaz actual se entra por **Artifacts → New from a template → Data** y se elige **Dashboard**. Si la herramienta disponible solo crea Docs/Pages, usa la interfaz de claude.ai para el Dashboard; no lo des por equivalente.
4. Verifica números, periodos, vínculos, hora de actualización y privacidad. Revisa el resultado junto con el usuario **antes de compartirlo**.
5. Guarda la URL privada del Dashboard en `reporte_url` de `config.toml`. Las actualizaciones siguientes deben conservar esa URL.
6. Después de la revisión conjunta, usa Share de claude.ai para dar acceso al jefe. Comprueba que el destino tenga acceso; un enlace privado no concede permisos por sí solo.

`diario equipo --publicar` y **Sync reporte** refrescan los datos locales. Mientras no se verifique una API de Dashboard en headless, devuelven el aviso de publicación pendiente y dejan preparado `publicar-prompt.txt`. Completa la publicación desde `/diario-publicar` o la interfaz de claude.ai. No hay publicación externa automática implementada todavía. Si falla publicar, conserva la URL anterior y repite solo la publicación tras revisar los datos; no crees artifacts duplicados.

## Ejecutar a las 7:00

```sh
cargo build --release
./target/release/diario instalar
```

El instalador genera `~/Library/LaunchAgents/com.diario.servidor.plist` y `com.diario.manana.plist` desde `launchd/`, con rutas absolutas. Mantiene el servidor y programa `todo` lunes a viernes a las 7:00. **launchd usa la zona del sistema**: el instalador comprueba `/etc/localtime` y exige `America/La_Paz`. `TZ` por sí solo no cambia la interpretación de `StartCalendarInterval`. La Mac debe estar encendida y con la sesión abierta; si duerme, launchd puede ejecutar una tarea vencida al despertar.

Comprueba:

```sh
launchctl print gui/$(id -u)/com.diario.servidor
launchctl print gui/$(id -u)/com.diario.manana
```

Logs: `salida/logs/`. Tras recompilar en otra ruta, repite instalar. Para detener:

```sh
launchctl bootout gui/$(id -u)/com.diario.servidor
launchctl bootout gui/$(id -u)/com.diario.manana
```

## Verificación

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Las pruebas cubren alertas y umbrales, Honeybadger, Slack, día hábil, exclusión entre procesos, SQL fijo, URLs/escape, demo por etapas, conservación del último HTML y acciones HTTP. La prueba real de conexiones, las cifras de producción y la publicación requieren evidencia adicional; los tests de fixtures no las sustituyen.
