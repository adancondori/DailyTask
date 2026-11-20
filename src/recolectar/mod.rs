pub mod claude;
pub mod clima;
pub mod dolar;
pub mod github;
mod jira;
pub mod slack;
use crate::{alertas, archivos, config::Config, modelo::*};
use anyhow::{bail, Result};
use chrono::{Duration, NaiveDate, Utc};
use std::{collections::BTreeMap, path::Path, time::Duration as Timeout};

pub fn http() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Timeout::from_secs(25))
        .user_agent("diario/0.1 (personal daily newspaper)")
        .build()
}
pub fn progreso(
    base: &Path,
    etapa: &str,
    fuente: Option<(&str, &str)>,
    corriendo: bool,
    error: Option<String>,
) -> Result<()> {
    let path = base.join("progreso.json");
    let mut p: Progreso = archivos::leer(&path).unwrap_or(Progreso {
        corriendo,
        etapa: etapa.into(),
        fuentes: BTreeMap::new(),
        error: None,
        actualizado: Utc::now(),
    });
    if etapa == "inicio" {
        p.fuentes.clear();
    }
    p.corriendo = corriendo;
    p.etapa = etapa.into();
    p.error = error;
    p.actualizado = Utc::now();
    if let Some((f, s)) = fuente {
        p.fuentes.insert(f.into(), s.into());
    }
    archivos::guardar(&path, &p)
}
fn obtener<T>(c: &Config, nombre: &str, run: impl FnOnce() -> Result<Fuente<T>>) -> Fuente<T> {
    let _ = progreso(
        &c.salida,
        "recolectar",
        Some((nombre, "En curso")),
        true,
        None,
    );
    let f = run().unwrap_or_else(Fuente::fallo);
    let status = if f.datos.is_none() {
        "No disponible"
    } else if !f.completo || f.error.is_some() {
        "Incompleto"
    } else {
        "Listo"
    };
    let detalle = f
        .error
        .as_ref()
        .map(|e| format!("{status}: {e}"))
        .unwrap_or_else(|| status.to_string());
    eprintln!("{nombre}: {detalle}");
    let _ = progreso(
        &c.salida,
        "recolectar",
        Some((nombre, &detalle)),
        true,
        None,
    );
    f
}
pub fn ejecutar(c: &Config, fecha: NaiveDate, demo: bool) -> Result<Datos> {
    if !demo && fecha != archivos::hoy() {
        bail!("La recolección real solo obtiene el día actual; las etapas posteriores sí permiten --fecha");
    }
    let mut d = if demo {
        demo_datos(fecha)?
    } else {
        let ayer = archivos::anterior_habil(fecha);
        Datos {
            version: 1,
            usuario: c.github_usuario.clone(),
            fecha,
            ayer_habil: ayer,
            demo: false,
            github: obtener(c, "GitHub", || {
                let g = github::obtener(c, fecha, ayer)?;
                let mut f = Fuente::ok(
                    g,
                    vec![
                        "gh api graphql: PRs org, autor y review-requested; REST commits paginados"
                            .into(),
                    ],
                );
                f.completo = f.datos.as_ref().unwrap().completo;
                Ok(f)
            }),
            clima: obtener(c, "Clima", || {
                Ok(Fuente::ok(
                    clima::obtener(c)?,
                    vec!["Open-Meteo forecast HTTP".into()],
                ))
            }),
            dolar: obtener(c, "Dólar", || {
                Ok(Fuente::ok(
                    dolar::obtener(c, fecha)?,
                    vec![dolar::BCB.into(), dolar::BINANCE.into()],
                ))
            }),
            gmail: obtener(c, "Gmail", || claude::gmail(c, fecha, ayer)),
            calendario: obtener(c, "Calendar", || claude::calendario(c, fecha, ayer)),
            slack: obtener(c, "Slack", || slack::obtener(c, fecha, ayer)),
            jira: obtener(c, "Jira", || claude::jira(c, fecha)),
            noticias: obtener(c, "Noticias", || claude::noticias(c, fecha)),
            alertas: vec![],
            hecho_ayer: vec![],
        }
    };
    terminar(c, &mut d)?;
    Ok(d)
}
fn terminar(c: &Config, d: &mut Datos) -> Result<()> {
    d.usuario = c.github_usuario.clone();
    d.alertas.clear();
    if let (Some(j), Some(g)) = (&d.jira.datos, &d.github.datos) {
        d.alertas = alertas::calcular(
            j,
            g,
            if d.demo {
                d.fecha.and_hms_opt(15, 0, 0).unwrap().and_utc()
            } else {
                Utc::now()
            },
        );
    }
    if let Some(g) = &mut d.gmail.datos {
        for m in &mut g.correos {
            m.item.sospechoso |=
                alertas::sospechoso(&format!("{} {}", m.item.titulo, m.item.detalle));
        }
    }
    if let Some(s) = &mut d.slack.datos {
        for m in &mut s.mensajes {
            m.item.sospechoso |=
                alertas::sospechoso(&format!("{} {}", m.item.titulo, m.item.detalle));
        }
    }
    d.hecho_ayer = alertas::hecho_ayer(d, c);
    archivos::guardar(&archivos::dia(&c.salida, d.fecha).join("datos.json"), &d)
}
pub fn actualizar(c: &Config, fecha: NaiveDate, demo: bool, nombre: &str) -> Result<Datos> {
    if ![
        "github", "clima", "dolar", "gmail", "calendar", "slack", "jira", "noticias",
    ]
    .contains(&nombre)
    {
        bail!("Fuente desconocida: {nombre}");
    }
    if demo {
        return ejecutar(c, fecha, true);
    }
    if fecha != archivos::hoy() {
        bail!("Solo se pueden actualizar fuentes del día actual");
    }
    let mut d: Datos = archivos::leer(&archivos::dia(&c.salida, fecha).join("datos.json"))?;
    let ayer = d.ayer_habil;
    match nombre {
        "github" => {
            d.github = obtener(c, "GitHub", || {
                let g = github::obtener(c, fecha, ayer)?;
                let completo = g.completo;
                let mut f = Fuente::ok(
                    g,
                    vec![
                        "gh: búsquedas divididas por fecha y paginadas; commits por repositorio"
                            .into(),
                    ],
                );
                f.completo = completo;
                Ok(f)
            })
        }
        "clima" => {
            d.clima = obtener(c, "Clima", || {
                Ok(Fuente::ok(
                    clima::obtener(c)?,
                    vec!["Open-Meteo HTTP".into()],
                ))
            })
        }
        "dolar" => {
            d.dolar = obtener(c, "Dólar", || {
                Ok(Fuente::ok(
                    dolar::obtener(c, fecha)?,
                    vec![dolar::BCB.into(), dolar::BINANCE.into()],
                ))
            })
        }
        "gmail" => d.gmail = obtener(c, "Gmail", || claude::gmail(c, fecha, ayer)),
        "calendar" => d.calendario = obtener(c, "Calendar", || claude::calendario(c, fecha, ayer)),
        "slack" => d.slack = obtener(c, "Slack", || slack::obtener(c, fecha, ayer)),
        "jira" => d.jira = obtener(c, "Jira", || claude::jira(c, fecha)),
        "noticias" => d.noticias = obtener(c, "Noticias", || claude::noticias(c, fecha)),
        _ => unreachable!(),
    }
    terminar(c, &mut d)?;
    Ok(d)
}
pub fn demo_datos(fecha: NaiveDate) -> Result<Datos> {
    let mut v: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/datos.json"))?;
    let base = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
    let delta = fecha.signed_duration_since(base);
    fn mover(v: &mut serde_json::Value, delta: Duration) {
        match v {
            serde_json::Value::String(s) => {
                if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
                    *s = (t + delta).to_rfc3339();
                } else if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
                    *s = (d + delta).to_string();
                }
            }
            serde_json::Value::Array(a) => {
                for x in a {
                    mover(x, delta)
                }
            }
            serde_json::Value::Object(o) => {
                for x in o.values_mut() {
                    mover(x, delta)
                }
            }
            _ => {}
        }
    }
    mover(&mut v, delta);
    v["fecha"] = fecha.to_string().into();
    v["ayer_habil"] = archivos::anterior_habil(fecha).to_string().into();
    Ok(serde_json::from_value(v)?)
}
