use super::{Fallos, Fault};
use crate::{archivos, config::Config, modelo::Fuente, recolectar};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::{Duration as Timeout, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
struct Registro {
    id: String,
    clase: String,
    url: String,
    creado: DateTime<Utc>,
    actualizado: DateTime<Utc>,
    resuelto: bool,
    ignorado: bool,
    avisos: u64,
}
#[derive(Serialize, Deserialize)]
struct Dia {
    fecha: NaiveDate,
    actualizado: DateTime<Utc>,
    registros: Vec<Registro>,
}
struct Cliente {
    agente: ureq::Agent,
    auth: String,
    proyecto: String,
    inicio: Instant,
    timeout: u64,
    limite: PathBuf,
}
impl Cliente {
    fn nuevo(c: &Config) -> Result<Self> {
        let limite = c.salida.join("equipo/honeybadger/limite.json");
        let privado = if c.mcp_config.is_empty() {
            PathBuf::from(std::env::var("HOME")?).join(".claude.json")
        } else {
            PathBuf::from(&c.mcp_config)
        };
        let obtener = |key: &str| -> Result<String> {
            if let Ok(v) = std::env::var(key) {
                if !v.is_empty() {
                    return Ok(v);
                }
            }
            let data: Value = serde_json::from_slice(
                &std::fs::read(&privado)
                    .context("No se pudo acceder a la configuración MCP privada de Honeybadger")?,
            )?;
            data["mcpServers"]["honeybadger"]["env"][key]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .with_context(|| format!("Falta {key} en el entorno o MCP existente"))
        };
        let key = obtener("HONEYBADGER_API_KEY")?;
        let proyecto = obtener("HONEYBADGER_PROJECT_ID")?;
        if !proyecto.chars().all(|c| c.is_ascii_digit()) {
            bail!("HONEYBADGER_PROJECT_ID no tiene formato numérico");
        }
        Ok(Self {
            agente: ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Timeout::from_secs(25))
                .user_agent("diario/0.1 (read-only)")
                .build(),
            auth: format!("Basic {}", STANDARD.encode(format!("{key}:"))),
            proyecto,
            inicio: Instant::now(),
            timeout: c.timeout_segundos,
            limite,
        })
    }
    fn leer(&self, u: &str) -> Result<Value> {
        comprobar_limite(&self.limite)?;
        if self.inicio.elapsed() > Timeout::from_secs(self.timeout) {
            bail!("Honeybadger agotó el tiempo; los días ya guardados se reutilizarán");
        }
        let respuesta = self
            .agente
            .get(u)
            .set("Authorization", &self.auth)
            .set("Accept", "application/json")
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(s, r) => {
                    if r.header("X-RateLimit-Remaining")==Some("0") {
                        if let Some(reset)=r.header("X-RateLimit-Reset").and_then(|v|v.parse::<i64>().ok()) {
                            let _=archivos::guardar(&self.limite,&reset);
                            return anyhow::anyhow!("Honeybadger agotó su límite horario; próxima lectura a partir de {}. Los días guardados se conservan",fecha_reset(reset));
                        }
                    }
                    anyhow::anyhow!("Honeybadger respondió HTTP {s}")
                },
                _ => anyhow::anyhow!("No se pudo leer Honeybadger por HTTP"),
            })?;
        if respuesta.header("X-RateLimit-Remaining") == Some("0") {
            if let Some(reset) = respuesta
                .header("X-RateLimit-Reset")
                .and_then(|v| v.parse::<i64>().ok())
            {
                archivos::guardar(&self.limite, &reset)?;
            }
        }
        let value: Value = respuesta
            .into_json()
            .context("Honeybadger devolvió JSON inválido")?;
        Ok(value)
    }

    fn estado(&self, id: &str) -> Result<EstadoObservado> {
        anyhow::ensure!(
            !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()),
            "ID Honeybadger inválido"
        );
        let v = self.leer(&format!(
            "https://app.honeybadger.io/v2/projects/{}/faults/{id}",
            self.proyecto
        ))?;
        anyhow::ensure!(
            v["environment"] == "production",
            "Honeybadger devolvió un estado fuera de producción"
        );
        Ok(EstadoObservado {
            actualizado: Utc::now(),
            resuelto: v["resolved"]
                .as_bool()
                .context("Falta resolved en Honeybadger")?,
            ignorado: v["ignored"]
                .as_bool()
                .context("Falta ignored en Honeybadger")?,
        })
    }

    fn paginas(
        &self,
        q: &str,
        inicio: Option<DateTime<Utc>>,
        fin: Option<DateTime<Utc>>,
    ) -> Result<Vec<Registro>> {
        let mut url = url::Url::parse(&format!(
            "https://app.honeybadger.io/v2/projects/{}/faults",
            self.proyecto
        ))?;
        {
            let mut qs = url.query_pairs_mut();
            qs.append_pair("q", q)
                .append_pair("order", "recent")
                .append_pair("limit", "25");
            if let Some(i) = inicio {
                qs.append_pair("occurred_after", &i.timestamp().to_string());
            }
            if let Some(f) = fin {
                qs.append_pair("occurred_before", &f.timestamp().to_string());
            }
        }
        let mut siguiente = Some(url.to_string());
        let mut visitadas = BTreeSet::new();
        let mut registros = BTreeMap::new();
        while let Some(u) = siguiente {
            comprobar_limite(&self.limite)?;
            if self.inicio.elapsed() > Timeout::from_secs(self.timeout) {
                bail!("Honeybadger agotó el tiempo; los días ya guardados se reutilizarán");
            }
            if !url_permitida(&u, &self.proyecto) || !visitadas.insert(u.clone()) {
                bail!("Paginación Honeybadger rechazada: origen o cursor inválido");
            }
            let value = self.leer(&u)?;
            let results = value["results"]
                .as_array()
                .context("Falta results en Honeybadger")?;
            for fault in results {
                if fault["environment"] != "production" {
                    bail!("Honeybadger no respetó el filtro de producción");
                }
                if !es_pago(fault) {
                    continue;
                }
                let r = normalizar(fault, &self.proyecto, inicio.is_some())?;
                registros.insert(r.id.clone(), r);
            }
            siguiente = value["links"]["next"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|href| url::Url::parse(&u)?.join(href).map(|url| url.to_string()))
                .transpose()?;
        }
        Ok(registros.into_values().collect())
    }
}
fn url_permitida(u: &str, proyecto: &str) -> bool {
    url::Url::parse(u).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str() == Some("app.honeybadger.io")
            && u.port_or_known_default() == Some(443)
            && u.username().is_empty()
            && u.password().is_none()
            && u.path() == format!("/v2/projects/{proyecto}/faults")
    })
}
fn es_pago(v: &Value) -> bool {
    let texto = format!(
        "{} {} {} {}",
        v["klass"].as_str().unwrap_or_default(),
        v["component"].as_str().unwrap_or_default(),
        v["action"].as_str().unwrap_or_default(),
        v["tags"]
    )
    .to_lowercase();
    [
        "payment",
        "capture",
        "refund",
        "renew",
        "membership",
        "billing",
        "stripe",
        "card_connect",
        "cardconnect",
        "pixel_pay",
        "pixelpay",
        "azul",
        "onepay",
        "one_pay",
        "adyen",
        "lugia",
        "xendit",
        "bac::",
        "pay_code",
        "pay_fast",
        "bolt",
        "user_token",
    ]
    .iter()
    .any(|p| texto.contains(p))
}
fn normalizar(v: &Value, proyecto: &str, ventana: bool) -> Result<Registro> {
    let id = v["id"]
        .as_u64()
        .map(|n| n.to_string())
        .or_else(|| v["id"].as_str().map(str::to_owned))
        .context("Fault sin id")?;
    if !id.chars().all(|c| c.is_ascii_digit()) {
        bail!("Identificador Honeybadger inválido");
    }
    let fecha = |k: &str| -> Result<DateTime<Utc>> {
        Ok(DateTime::parse_from_rfc3339(
            v[k].as_str()
                .with_context(|| format!("Falta {k} del fault"))?,
        )?
        .with_timezone(&Utc))
    };
    Ok(Registro {
        id: id.clone(),
        clase: v["klass"]
            .as_str()
            .filter(|s| !s.is_empty())
            .context("Fault sin clase")?
            .into(),
        url: format!("https://app.honeybadger.io/projects/{proyecto}/faults/{id}"),
        creado: fecha("created_at")?,
        actualizado: fecha("last_notice_at")?,
        resuelto: v["resolved"]
            .as_bool()
            .context("Fault sin estado resolved")?,
        ignorado: v["ignored"].as_bool().unwrap_or(false),
        avisos: if ventana {
            v["notices_count_in_range"].as_u64().context("Honeybadger no expuso el conteo exacto de la ventana; no se usa el total histórico")?
        } else {
            0
        },
    })
}
#[derive(Serialize, Deserialize)]
struct EstadoObservado {
    actualizado: DateTime<Utc>,
    resuelto: bool,
    ignorado: bool,
}

fn fecha_reset(reset: i64) -> String {
    DateTime::from_timestamp(reset, 0)
        .map(|f| {
            f.with_timezone(&chrono::FixedOffset::west_opt(4 * 3600).unwrap())
                .format("%d/%m %H:%M:%S UTC-4")
                .to_string()
        })
        .unwrap_or_else(|| reset.to_string())
}

fn comprobar_limite(path: &std::path::Path) -> Result<()> {
    if path.is_file() {
        let reset: i64 = archivos::leer(path)?;
        if reset > Utc::now().timestamp() {
            bail!("Honeybadger agotó su límite horario; próxima lectura a partir de {}. Los días guardados se conservan",fecha_reset(reset));
        }
    }
    Ok(())
}

pub fn obtener(c: &Config, fecha: NaiveDate) -> Result<Fuente<Fallos>> {
    let cliente = Cliente::nuevo(c)?;
    let instante = |d: NaiveDate| d.and_hms_opt(4, 0, 0).unwrap().and_utc();
    let mut familias: BTreeMap<String, Fault> = BTreeMap::new();
    let cache = c.salida.join("equipo/honeybadger");
    for n in 1..=60 {
        let dia = fecha - Duration::days(n);
        let path = cache.join(format!("{dia}.json"));
        let datos: Dia = if path.is_file() {
            archivos::leer(&path)?
        } else {
            recolectar::progreso(
                &c.salida,
                "equipo",
                Some(("Honeybadger", &format!("Consultando {dia}"))),
                true,
                None,
            )?;
            let registros = cliente.paginas(
                "environment:production",
                Some(instante(dia)),
                Some(instante(dia + Duration::days(1))),
            )?;
            let d = Dia {
                fecha: dia,
                actualizado: Utc::now(),
                registros,
            };
            archivos::guardar(&path, &d)?;
            d
        };
        if datos.fecha != dia {
            bail!("Cache Honeybadger fuera de fecha");
        }
        for registro in datos.registros {
            let meta = &registro;
            let f = familias.entry(meta.id.clone()).or_insert_with(|| Fault {
                id: meta.id.clone(),
                clase: meta.clase.clone(),
                url: meta.url.clone(),
                creado: meta.creado,
                actualizado: datos.actualizado,
                cerrado: None,
                resuelto: meta.resuelto,
                ignorado: meta.ignorado,
                avisos_por_dia: BTreeMap::new(),
                pr_cierre: None,
            });
            if datos.actualizado > f.actualizado {
                f.actualizado = datos.actualizado;
                f.resuelto = meta.resuelto;
                f.ignorado = meta.ignorado;
            }
            f.avisos_por_dia.insert(dia, registro.avisos);
        }
    }
    let hoy = crate::archivos::hoy();
    let path_estados = cache.join("estados").join(format!("{hoy}.json"));
    let mut estados: BTreeMap<String, EstadoObservado> = if path_estados.is_file() {
        archivos::leer(&path_estados)?
    } else {
        BTreeMap::new()
    };
    let mut error_estado = None;
    let off = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
    for f in familias.values_mut() {
        if !estados.contains_key(&f.id) {
            let estado = if f.actualizado.with_timezone(&off).date_naive() == hoy {
                Ok(EstadoObservado {
                    actualizado: f.actualizado,
                    resuelto: f.resuelto,
                    ignorado: f.ignorado,
                })
            } else {
                cliente.estado(&f.id)
            };
            match estado {
                Ok(e) => {
                    estados.insert(f.id.clone(), e);
                    archivos::guardar(&path_estados, &estados)?;
                }
                Err(e) => {
                    error_estado = Some(format!(
                        "Conteos diarios completos; estados parcialmente actualizados: {e:#}"
                    ));
                    break;
                }
            }
        }
        if let Some(e) = estados.get(&f.id) {
            f.actualizado = e.actualizado;
            f.resuelto = e.resuelto;
            f.ignorado = e.ignorado;
        }
    }
    recolectar::progreso(
        &c.salida,
        "equipo",
        Some(("Honeybadger", "Listo")),
        true,
        None,
    )?;
    let mut fuente=Fuente::ok(Fallos{familias:familias.into_values().collect()},vec!["Honeybadger API v2 GET, familias con actividad en los últimos sesenta días completos, producción, ventanas UTC-4, todas las páginas".into(),"Clasificación por clase/componente/acción/tags de pagos; conteos exactos notices_count_in_range; días guardados sin reconsulta".into(),"Estados: cache diario separado de conteos; se refrescan por id los errores no leídos hoy, sin volver a consultar su historia".into(),"El estado de cada error es el observado en su última lectura, fechado en actualizado; no se presume un estado actual para registros históricos".into(),"La API no expone fecha de resolución ni PR de cierre: permanecen desconocidos; no se infieren del último aviso o deploy".into()]);
    fuente.completo = error_estado.is_none();
    fuente.error = error_estado;
    Ok(fuente)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn no_filtrar_credenciales_por_url() {
        assert!(url_permitida(
            "https://app.honeybadger.io/v2/projects/1/faults?page=2",
            "1"
        ));
        for u in [
            "https://evil.test/v2/projects/1/faults",
            "https://app.honeybadger.io/v2/projects/2/faults",
            "http://app.honeybadger.io/v2/projects/1/faults",
            "https://key@app.honeybadger.io/v2/projects/1/faults",
        ] {
            assert!(!url_permitida(u, "1"));
        }
    }
    #[test]
    fn conteo_de_ventana_y_filtro() {
        let mut v = json!({"id":1,"klass":"CaptureFailedError","component":"CapturePaymentJob","environment":"production","created_at":"2026-01-01T00:00:00Z","last_notice_at":"2026-11-19T14:00:00Z","resolved":true,"ignored":false,"notices_count":999,"notices_count_in_range":3});
        assert!(es_pago(&v));
        let r = normalizar(&v, "1", true).unwrap();
        assert_eq!(r.avisos, 3);
        assert!(r.resuelto);
        v.as_object_mut().unwrap().remove("notices_count_in_range");
        assert!(normalizar(&v, "1", true).is_err());
        assert!(!es_pago(
            &json!({"klass":"SitemapError","component":"SitemapJob","tags":[]})
        ));
    }
}
