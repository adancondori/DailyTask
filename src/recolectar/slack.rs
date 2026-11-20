use crate::{config::Config, modelo::*};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use regex::Regex;
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration as Espera, Instant},
};

const METODOS: &[&str] = &[
    "auth.test",
    "conversations.list",
    "conversations.history",
    "conversations.replies",
    "search.messages",
    "users.info",
];

struct Cliente {
    agente: ureq::Agent,
    token: String,
    cookie: String,
    inicio: Instant,
    timeout: u64,
    llamadas: Cell<u64>,
    usuarios: RefCell<BTreeMap<String, String>>,
}

impl Cliente {
    fn nuevo(c: &Config) -> Result<Option<Self>> {
        let privado = if c.mcp_config.is_empty() {
            PathBuf::from(std::env::var("HOME")?).join(".claude.json")
        } else {
            PathBuf::from(&c.mcp_config)
        };
        let data: Value = std::fs::read(privado)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or(Value::Null);
        let env = data["mcpServers"].as_object().and_then(|m| {
            m.values().find_map(|s| {
                s.get("env")
                    .filter(|e| e.get("SLACK_MCP_XOXC_TOKEN").is_some())
            })
        });
        let valor = |nombre: &str| {
            std::env::var(nombre)
                .ok()
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    env.and_then(|e| e[nombre].as_str())
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
        };
        let (Some(token), Some(cookie)) =
            (valor("SLACK_MCP_XOXC_TOKEN"), valor("SLACK_MCP_XOXD_TOKEN"))
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            agente: ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Espera::from_secs(25))
                .user_agent("diario/0.1 (read-only)")
                .build(),
            token,
            cookie,
            inicio: Instant::now(),
            timeout: c.timeout_segundos,
            llamadas: Cell::new(0),
            usuarios: RefCell::new(BTreeMap::new()),
        }))
    }
    fn get(&self, metodo: &str, parametros: &[(&str, String)]) -> Result<Value> {
        anyhow::ensure!(
            METODOS.contains(&metodo),
            "Método Slack fuera de la lista de lectura"
        );
        let mut url = url::Url::parse(&format!("https://slack.com/api/{metodo}"))?;
        url.query_pairs_mut()
            .extend_pairs(parametros.iter().map(|(k, v)| (*k, v.as_str())));
        for intento in 0..3 {
            if self.inicio.elapsed().as_secs() >= self.timeout {
                bail!("Slack agotó el tiempo de lectura");
            }
            self.llamadas.set(self.llamadas.get() + 1);
            match self
                .agente
                .get(url.as_str())
                .set("Authorization", &format!("Bearer {}", self.token))
                .set("Cookie", &format!("d={}", self.cookie))
                .set("Accept", "application/json")
                .call()
            {
                Ok(r) => {
                    let v: Value = r.into_json().context("Slack devolvió JSON inválido")?;
                    if v["ok"] != true {
                        bail!(
                            "Slack {metodo}: {}",
                            v["error"].as_str().unwrap_or("respuesta sin ok")
                        );
                    }
                    return Ok(v);
                }
                Err(ureq::Error::Status(429, r)) => {
                    let espera = r
                        .header("Retry-After")
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(10);
                    if espera > 60 || self.inicio.elapsed().as_secs() + espera >= self.timeout {
                        bail!("Slack alcanzó su límite; reintentar después de {espera} segundos");
                    }
                    std::thread::sleep(Espera::from_secs(espera));
                }
                Err(ureq::Error::Status(s, _)) if s >= 500 && intento < 2 => {
                    std::thread::sleep(Espera::from_secs(1 << intento))
                }
                Err(ureq::Error::Status(s, _)) => bail!("Slack {metodo}: HTTP {s}"),
                Err(_) if intento < 2 => std::thread::sleep(Espera::from_secs(1 << intento)),
                Err(_) => bail!("No se pudo leer Slack {metodo}"),
            }
        }
        bail!("Slack agotó los reintentos de lectura")
    }
    fn paginas(
        &self,
        metodo: &str,
        clave: &str,
        mut params: Vec<(&str, String)>,
    ) -> Result<Vec<Value>> {
        params.push(("limit", "200".into()));
        let mut cursor = String::new();
        let mut vistos = BTreeSet::new();
        let mut filas = vec![];
        loop {
            let mut p = params.clone();
            if !cursor.is_empty() {
                p.push(("cursor", cursor.clone()));
            }
            let v = self.get(metodo, &p)?;
            filas.extend(
                v[clave]
                    .as_array()
                    .context("Slack no devolvió la colección esperada")?
                    .iter()
                    .cloned(),
            );
            let next = v["response_metadata"]["next_cursor"]
                .as_str()
                .unwrap_or_default()
                .trim();
            if next.is_empty() {
                anyhow::ensure!(v["has_more"] != true, "Slack indicó más páginas sin cursor");
                break;
            }
            anyhow::ensure!(vistos.insert(next.to_owned()), "Cursor Slack repetido");
            cursor = next.to_owned();
        }
        Ok(filas)
    }
    fn buscar(&self, q: &str) -> Result<Vec<Value>> {
        let mut filas = vec![];
        let mut pagina = 1;
        loop {
            let v = self.get(
                "search.messages",
                &[
                    ("query", q.into()),
                    ("count", "100".into()),
                    ("page", pagina.to_string()),
                    ("sort", "timestamp".into()),
                    ("sort_dir", "desc".into()),
                ],
            )?;
            let m = &v["messages"];
            let paginas = m["paging"]["pages"]
                .as_u64()
                .context("Slack search no expone paginación")?;
            anyhow::ensure!(
                paginas <= 100,
                "La búsqueda Slack supera el límite de páginas; cobertura incompleta"
            );
            filas.extend(
                m["matches"]
                    .as_array()
                    .context("Faltan coincidencias Slack")?
                    .iter()
                    .cloned(),
            );
            if pagina >= paginas {
                break;
            }
            pagina += 1;
        }
        Ok(filas)
    }
    fn nombre(&self, id: &str) -> String {
        if let Some(n) = self.usuarios.borrow().get(id) {
            return n.clone();
        }
        let nombre = self
            .get("users.info", &[("user", id.into())])
            .ok()
            .and_then(|v| {
                v["user"]["real_name"]
                    .as_str()
                    .or(v["user"]["name"].as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| id.to_owned());
        self.usuarios.borrow_mut().insert(id.into(), nombre.clone());
        nombre
    }
}

fn fecha(ts: &str) -> Result<DateTime<Utc>> {
    let (segundos, fraccion) = ts.split_once('.').unwrap_or((ts, "0"));
    anyhow::ensure!(
        !segundos.is_empty()
            && segundos.bytes().all(|b| b.is_ascii_digit())
            && fraccion.len() <= 9
            && fraccion.bytes().all(|b| b.is_ascii_digit()),
        "Timestamp Slack inválido"
    );
    let nanos = format!("{fraccion:0<9}").parse::<u32>()?;
    DateTime::from_timestamp(segundos.parse()?, nanos).context("Fecha Slack fuera de rango")
}

fn produccion(texto: &str) -> bool {
    let t = texto.to_lowercase();
    let entorno = ["production", "producción", "produccion"]
        .iter()
        .any(|s| t.contains(s));
    let exito = [
        "success",
        "deployed",
        "desplegado",
        "completado",
        ":white_check_mark:",
        "✅",
    ]
    .iter()
    .any(|s| t.contains(s));
    entorno
        && exito
        && ![
            "deployment queue",
            "deployment preparation",
            "pendiente",
            "failed",
            "failure",
            "falló",
        ]
        .iter()
        .any(|s| t.contains(s))
}

fn alerta_pendiente(texto: &str) -> bool {
    let t = texto.to_lowercase();
    ["state: alarm", "state: firing", "status: triggered"]
        .iter()
        .any(|s| t.contains(s))
        && !["resolved", "recovered", "resuelto", "recuperado"]
            .iter()
            .any(|s| t.contains(s))
}

fn texto_mensaje(v: &Value) -> String {
    fn bloque(v: &Value, partes: &mut Vec<String>, profundidad: usize) {
        if profundidad > 16 {
            return;
        }
        if let Some(texto) = v["text"].as_str() {
            partes.push(texto.into());
        } else if v["text"].is_object() {
            bloque(&v["text"], partes, profundidad + 1);
        }
        if v["type"] == "link" {
            if let Some(url) = v["url"].as_str() {
                partes.push(url.into());
            }
        }
        for clave in ["elements", "fields", "blocks"] {
            if let Some(xs) = v[clave].as_array() {
                for x in xs {
                    bloque(x, partes, profundidad + 1);
                }
            }
        }
    }
    let mut partes = vec![v["text"].as_str().unwrap_or_default().to_owned()];
    if let Some(adjuntos) = v["attachments"].as_array() {
        for a in adjuntos {
            for clave in ["pretext", "title", "text", "fallback", "title_link"] {
                if let Some(texto) = a[clave].as_str() {
                    partes.push(texto.into());
                }
            }
            if let Some(campos) = a["fields"].as_array() {
                for campo in campos {
                    partes.push(format!(
                        "{}: {}",
                        campo["title"].as_str().unwrap_or_default(),
                        campo["value"].as_str().unwrap_or_default()
                    ));
                }
            }
            if let Some(xs) = a["blocks"].as_array() {
                for x in xs {
                    bloque(x, &mut partes, 0);
                }
            }
        }
    }
    if let Some(xs) = v["blocks"].as_array() {
        for x in xs {
            bloque(x, &mut partes, 0);
        }
    }
    let mut vistos = BTreeSet::new();
    partes.retain(|s| !s.trim().is_empty() && vistos.insert(s.clone()));
    partes.join("\n")
}

fn raiz(v: &Value) -> String {
    v["thread_ts"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            v["permalink"]
                .as_str()
                .and_then(|s| url::Url::parse(s).ok())
                .and_then(|u| {
                    u.query_pairs()
                        .find(|(k, _)| k == "thread_ts")
                        .map(|(_, v)| v.into_owned())
                })
        })
        .unwrap_or_else(|| v["ts"].as_str().unwrap_or_default().into())
}

fn normalizar(
    cl: &Cliente,
    workspace: &str,
    canal: &str,
    nombre: &str,
    v: &Value,
) -> Result<Mensaje> {
    let ts = v["ts"].as_str().context("Mensaje Slack sin ts")?;
    let cuando = fecha(ts)?;
    anyhow::ensure!(
        canal.bytes().all(|b| b.is_ascii_alphanumeric()),
        "ID de canal Slack inválido"
    );
    let texto = texto_mensaje(v);
    let persona = v["user"]
        .as_str()
        .map(|u| cl.nombre(u))
        .unwrap_or_else(|| v["username"].as_str().unwrap_or("Bot").into());
    let sospechoso=Regex::new(r"(?i)(ignora|ignore|disregard).{0,60}(instrucciones|instructions|prompt)|system.?prompt|para (el asistente|la ia)")?.is_match(&texto);
    Ok(Mensaje {
        item: Item {
            id: format!("slack-{canal}-{ts}"),
            titulo: texto
                .lines()
                .next()
                .unwrap_or("Mensaje de Slack")
                .chars()
                .take(140)
                .collect(),
            detalle: texto.chars().take(4000).collect(),
            url: format!("{workspace}archives/{canal}/p{}", ts.replace('.', "")),
            persona,
            estado: Estado::Informativo,
            fecha: cuando,
            sospechoso,
        },
        canal: nombre.into(),
        mencion: false,
        dm: false,
        espera_respuesta: false,
        resuelto: true,
        deploy_prs: vec![],
        produccion: false,
    })
}

fn ultimo_pendiente<'a>(
    mensajes: &'a [Value],
    yo: &str,
    inicio: DateTime<Utc>,
    fin: DateTime<Utc>,
) -> Option<&'a Value> {
    mensajes
        .iter()
        .filter(|m| {
            m["user"].is_string()
                && m["bot_id"].is_null()
                && m["text"].as_str().is_some_and(|s| !s.is_empty())
                && m["ts"]
                    .as_str()
                    .and_then(|s| fecha(s).ok())
                    .is_some_and(|f| f >= inicio && f < fin)
        })
        .max_by_key(|m| m["ts"].as_str().and_then(|s| fecha(s).ok()))
        .filter(|m| m["user"] != yo)
}

fn hilo(
    cl: &Cliente,
    canal: &str,
    raiz: String,
    cache: &mut BTreeMap<(String, String), Vec<Value>>,
) -> Result<Vec<Value>> {
    let key = (canal.to_owned(), raiz.clone());
    if let Some(xs) = cache.get(&key) {
        return Ok(xs.clone());
    }
    let xs = cl.paginas(
        "conversations.replies",
        "messages",
        vec![("channel", canal.into()), ("ts", raiz)],
    )?;
    cache.insert(key, xs.clone());
    Ok(xs)
}

fn peticion(texto: &str) -> bool {
    let t = texto.to_lowercase();
    t.contains('?')
        || [
            "puedes",
            "podés",
            "podes",
            "can you",
            "could you",
            "por favor",
            "please",
            "necesito",
            "confirma",
            "revisa",
            "review",
            "pendiente",
            "avisas",
            "avísame",
            "avisame",
        ]
        .iter()
        .any(|s| t.contains(s))
}

fn pendientes(
    cl: &Cliente,
    c: &Config,
    workspace: &str,
    yo: &str,
    dia: NaiveDate,
    ayer: NaiveDate,
) -> Result<Vec<Mensaje>> {
    let inicio = ayer.and_hms_opt(4, 0, 0).unwrap().and_utc();
    let fin = (dia + Duration::days(1))
        .and_hms_opt(4, 0, 0)
        .unwrap()
        .and_utc();
    let ventana = format!(
        "after:{} before:{}",
        ayer - Duration::days(1),
        dia + Duration::days(2)
    );
    let menciones = cl.buscar(&format!("<@{yo}> {ventana} -in:{}", c.canal_deploy))?;
    let dms = cl.buscar(&format!("is:dm {ventana}"))?;
    let mut out = vec![];
    let mut cache = BTreeMap::new();
    for m in menciones {
        let canal = m["channel"]["id"].as_str().context("Mención sin canal")?;
        let nombre = m["channel"]["name"].as_str().unwrap_or(canal);
        if nombre == c.canal_deploy || c.canales_bot.iter().any(|s| s == nombre) || m["user"] == yo
        {
            continue;
        }
        let ts = m["ts"].as_str().context("Mención sin fecha")?;
        let cuando = fecha(ts)?;
        if cuando < inicio || cuando >= fin {
            continue;
        }
        let hilo = hilo(cl, canal, raiz(&m), &mut cache)?;
        let respondido = hilo.iter().any(|r| {
            r["user"] == yo
                && r["ts"]
                    .as_str()
                    .and_then(|s| fecha(s).ok())
                    .is_some_and(|f| f > cuando && f < fin)
        });
        if respondido {
            continue;
        }
        let mut x = normalizar(cl, workspace, canal, nombre, &m)?;
        x.mencion = true;
        x.espera_respuesta = true;
        x.resuelto = false;
        x.item.estado = Estado::Accion;
        out.push(x);
    }
    let propios = cl.buscar(&format!("from:{yo} {ventana} -in:{}", c.canal_deploy))?;
    for m in propios {
        let canal = m["channel"]["id"]
            .as_str()
            .context("Participación sin canal")?;
        let nombre = m["channel"]["name"].as_str().unwrap_or(canal);
        if canal.starts_with('D')
            || m["channel"]["is_im"] == true
            || m["channel"]["is_mpim"] == true
            || nombre.starts_with("mpdm-")
            || nombre == c.canal_deploy
            || c.canales_bot.iter().any(|s| s == nombre)
        {
            continue;
        }
        let xs = hilo(cl, canal, raiz(&m), &mut cache)?;
        if let Some(ultimo) = ultimo_pendiente(&xs, yo, inicio, fin)
            .filter(|m| peticion(m["text"].as_str().unwrap_or_default()))
        {
            let mut x = normalizar(cl, workspace, canal, nombre, ultimo)?;
            x.espera_respuesta = true;
            x.resuelto = false;
            x.item.estado = Estado::Accion;
            out.push(x);
        }
    }
    let canales = dms
        .iter()
        .filter_map(|m| m["channel"]["id"].as_str())
        .collect::<BTreeSet<_>>();
    for canal in canales {
        let mut mensajes = cl.paginas(
            "conversations.history",
            "messages",
            vec![
                ("channel", canal.into()),
                ("oldest", inicio.timestamp().to_string()),
                ("latest", fin.timestamp().to_string()),
                ("inclusive", "true".into()),
            ],
        )?;
        let raices = dms
            .iter()
            .filter(|m| {
                m["channel"]["id"] == canal && raiz(m) != m["ts"].as_str().unwrap_or_default()
            })
            .map(raiz)
            .chain(
                mensajes
                    .iter()
                    .filter(|m| m["reply_count"].as_u64().unwrap_or(0) > 0)
                    .map(raiz),
            )
            .collect::<BTreeSet<_>>();
        for r in raices {
            mensajes.extend(hilo(cl, canal, r, &mut cache)?);
        }
        let Some(ultimo) = ultimo_pendiente(&mensajes, yo, inicio, fin) else {
            continue;
        };
        let mut x = normalizar(cl, workspace, canal, canal, ultimo)?;
        x.dm = true;
        x.espera_respuesta = true;
        x.resuelto = false;
        x.item.estado = Estado::Accion;
        out.push(x);
    }
    Ok(out)
}

pub fn obtener(c: &Config, dia: NaiveDate, ayer: NaiveDate) -> Result<Fuente<Slack>> {
    let Some(cl) = Cliente::nuevo(c)? else {
        return super::claude::slack(c, dia, ayer);
    };
    let auth = cl.get("auth.test", &[])?;
    let yo = auth["user_id"]
        .as_str()
        .context("Slack no identificó al usuario autenticado")?;
    let workspace = auth["url"]
        .as_str()
        .context("Slack no devolvió URL de workspace")?;
    let origen = url::Url::parse(workspace)?;
    anyhow::ensure!(
        origen.scheme() == "https"
            && origen.host_str().is_some_and(|h| h.ends_with(".slack.com"))
            && origen.username().is_empty()
            && origen.password().is_none(),
        "Workspace Slack inválido"
    );
    let mut mensajes = vec![];
    let mut errores = vec![];
    match pendientes(&cl, c, workspace, yo, dia, ayer) {
        Ok(xs) => mensajes.extend(xs),
        Err(e) => errores.push(format!("Pendientes: {e:#}")),
    };
    let canales = cl.paginas(
        "conversations.list",
        "channels",
        vec![
            ("types", "public_channel,private_channel".into()),
            ("exclude_archived", "true".into()),
        ],
    )?;
    let prs = Regex::new(r"https://github\.com/([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)/pull/([0-9]+)")?;
    for nombre in c.canales_bot.iter().chain(std::iter::once(&c.canal_deploy)) {
        let resultado = (|| -> Result<Vec<Mensaje>> {
            let canal = canales
                .iter()
                .find(|v| v["name"] == nombre.as_str())
                .and_then(|v| v["id"].as_str())
                .with_context(|| format!("Canal #{nombre} no accesible"))?;
            let desde = if nombre == &c.canal_deploy {
                dia - Duration::days(61)
            } else {
                ayer
            };
            let inicio = desde.and_hms_opt(4, 0, 0).unwrap().and_utc();
            let fin = (dia + Duration::days(1))
                .and_hms_opt(4, 0, 0)
                .unwrap()
                .and_utc();
            let filas = cl.paginas(
                "conversations.history",
                "messages",
                vec![
                    ("channel", canal.into()),
                    ("oldest", inicio.timestamp().to_string()),
                    ("latest", fin.timestamp().to_string()),
                    ("inclusive", "true".into()),
                ],
            )?;
            let mut xs = vec![];
            for v in filas {
                let Some(f) = v["ts"].as_str().and_then(|s| fecha(s).ok()) else {
                    bail!("Slack devolvió un mensaje sin fecha válida")
                };
                if f < inicio || f >= fin {
                    continue;
                }
                let texto = texto_mensaje(&v);
                if nombre == &c.canal_deploy {
                    if !produccion(&texto) {
                        continue;
                    }
                    let enlaces = prs
                        .captures_iter(&texto)
                        .filter(|m| m[1].split('/').next() == Some(c.github_organizacion.as_str()))
                        .map(|m| m[0].to_owned())
                        .collect::<BTreeSet<_>>();
                    if enlaces.is_empty() {
                        continue;
                    }
                    let mut x = normalizar(&cl, workspace, canal, nombre, &v)?;
                    x.deploy_prs = enlaces.into_iter().collect();
                    x.produccion = true;
                    x.item.estado = Estado::Listo;
                    xs.push(x);
                } else {
                    let mut x = normalizar(&cl, workspace, canal, nombre, &v)?;
                    x.resuelto = !alerta_pendiente(&texto);
                    if !x.resuelto {
                        x.item.estado = Estado::Accion;
                    }
                    xs.push(x);
                }
            }
            Ok(xs)
        })();
        match resultado {
            Ok(xs) => mensajes.extend(xs),
            Err(e) => errores.push(format!("#{nombre}: {e:#}")),
        }
    }
    let mut vistos = BTreeSet::new();
    mensajes.retain(|m| vistos.insert(m.item.url.clone()));
    let mut f=Fuente::ok(Slack{mensajes},vec![format!("Slack Web API GET; {} lecturas, solo métodos autorizados; auth.test verificó la sesión existente",cl.llamadas.get()),format!("Pendientes: {ayer} 00:00 hasta fin de {dia}, UTC-4; menciones sin respuesta posterior, peticiones explícitas en hilos de participación y última intervención en DMs incluidos sus hilos; todas las páginas"),"Bots: mensajes originales contados por Rust; deployments excluido de pendientes, últimos sesenta y dos días, solo éxito explícito en producción y PR enlazado".into(),"No se marcan mensajes ni se envía contenido; no se infieren despliegues a partir de colas o anuncios de release sin confirmación de producción".into()]);
    f.completo = errores.is_empty();
    if !errores.is_empty() {
        f.error = Some(errores.join("; "));
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn respuesta_en_hilo_se_evalua_por_fecha_no_por_orden() {
        let mut xs = vec![
            serde_json::json!({"user":"yo","ts":"1795060800.000030","text":"respondido"}),
            serde_json::json!({"user":"otra","ts":"1795060800.000020","text":"pregunta?"}),
        ];
        let inicio = fecha("1795060800.000000").unwrap();
        let fin = fecha("1795060801.000000").unwrap();
        assert!(ultimo_pendiente(&xs, "yo", inicio, fin).is_none());
        xs.insert(
            0,
            serde_json::json!({"user":"otra","ts":"1795060800.000040","text":"puedes revisar?"}),
        );
        assert_eq!(
            ultimo_pendiente(&xs, "yo", inicio, fin).unwrap()["ts"],
            "1795060800.000040"
        );
        assert!(peticion("Puedes confirmarlo?"));
        assert!(!peticion("Gracias, quedó listo"));
    }
    #[test]
    fn no_confunde_cola_o_release_con_produccion() {
        assert!(!produccion(
            "Deployment Queue: production success pendiente"
        ));
        assert!(!produccion("NEW GREENFIELD RELEASE 2.0"));
        assert!(!produccion("production deployment failed"));
        assert!(produccion(
            "production deployment completed :white_check_mark:"
        ));
        assert_eq!(
            fecha("1795060800.000001").unwrap().timestamp_micros(),
            1795060800000001
        );
        assert!(fecha("1795060800.1/../../").is_err());
        assert!(alerta_pendiente("state: firing 🚨"));
        assert!(!alerta_pendiente("state: firing · resolved"));
        assert!(!alerta_pendiente(
            ":rotating_light: Rate limit hit: external api reads per bearer token"
        ));
    }
    #[test]
    fn conserva_alertas_estructuradas_sin_texto_principal() {
        let cl = Cliente {
            agente: ureq::AgentBuilder::new().build(),
            token: "prueba".into(),
            cookie: "prueba".into(),
            inicio: Instant::now(),
            timeout: 1,
            llamadas: Cell::new(0),
            usuarios: RefCell::new(BTreeMap::new()),
        };
        let v = serde_json::json!({"ts":"1795060800.000001","username":"Monitor de prueba","text":"","attachments":[{"title":"Servicio de prueba","fields":[{"title":"State","value":"firing"}]}]});
        let m = normalizar(&cl, "https://ejemplo.slack.com/", "C123", "prueba", &v).unwrap();
        assert!(m.item.detalle.contains("State: firing"));
        assert!(alerta_pendiente(&m.item.detalle));
        assert_eq!(cl.llamadas.get(), 0);
    }
}
