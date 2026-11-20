use crate::{alertas, archivos, config::Config, editar, modelo::*};
use anyhow::Result;
use chrono::{Duration, FixedOffset, NaiveDate, Utc};
use minijinja::{context, Environment};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub fn url_segura(s: &str) -> bool {
    url::Url::parse(s).is_ok_and(|u| {
        matches!(u.scheme(), "https" | "http")
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
    })
}
fn limpiar_urls(v: &mut Value) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                if (k == "url" || k.ends_with("_url")) && v.is_string() {
                    if !url_segura(v.as_str().unwrap()) {
                        *v = Value::String(String::new());
                    }
                } else {
                    limpiar_urls(v)
                }
            }
        }
        Value::Array(a) => {
            for x in a {
                limpiar_urls(x)
            }
        }
        _ => {}
    }
}
fn etiqueta(e: &Estado) -> (&'static str, &'static str) {
    match e {
        Estado::Accion => ("!", "accion"),
        Estado::EnCurso => ("◷", "curso"),
        Estado::Bloqueado => ("⊘", "bloqueado"),
        Estado::Listo => ("✓", "listo"),
        Estado::Informativo => ("ⓘ", "info"),
    }
}
pub fn entorno() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::Html);
    env.add_filter(
        "enlace",
        |s: String| if url_segura(&s) { s } else { String::new() },
    );
    env.add_filter("local", |s: String| {
        chrono::DateTime::parse_from_rfc3339(&s)
            .map(|d| {
                d.with_timezone(&FixedOffset::west_opt(4 * 3600).unwrap())
                    .format("%d/%m %H:%M")
                    .to_string()
            })
            .unwrap_or(s)
    });
    env.add_filter("numero", |n: f64| format!("{n:.2}"));
    env.add_filter("precision", |n: f64| format!("{n:.4}"));
    env.add_function("estado_info", |s: String| {
        let e: Estado = serde_json::from_value(json!(s)).unwrap_or(Estado::Informativo);
        let (icono, token) = etiqueta(&e);
        minijinja::Value::from_serialize(json!({"icono":icono,"token":token}))
    });
    env
}

fn actividad_agrupada(d: &Datos) -> Vec<Value> {
    let mut grupos = BTreeMap::<&str, Vec<&Item>>::new();
    for item in &d.hecho_ayer {
        let tipo = if item.id.starts_with("commit-") {
            "Commits"
        } else if item.id.starts_with("jira-") {
            "Tickets"
        } else if url::Url::parse(&item.url)
            .is_ok_and(|u| u.host_str() == Some("github.com") && u.path().contains("/pull/"))
        {
            "Pull requests"
        } else if d
            .calendario
            .datos
            .as_ref()
            .is_some_and(|c| c.eventos.iter().any(|e| e.item.id == item.id))
        {
            "Reuniones"
        } else {
            "Otras actividades"
        };
        grupos.entry(tipo).or_default().push(item);
    }
    [
        "Pull requests",
        "Tickets",
        "Commits",
        "Reuniones",
        "Otras actividades",
    ]
    .into_iter()
    .filter_map(|tipo| {
        let items = grupos.remove(tipo)?;
        let resumen = if tipo == "Pull requests" {
            let prs = items.iter().map(|i| &i.url).collect::<BTreeSet<_>>().len();
            let abiertos = items
                .iter()
                .filter(|i| i.titulo.starts_with("PR abierto:"))
                .count();
            let integrados = items
                .iter()
                .filter(|i| i.titulo.starts_with("PR mergeado:"))
                .count();
            let reviews = items
                .iter()
                .filter(|i| i.titulo.starts_with("Review:"))
                .count();
            format!(
                "{prs} PRs · abiertos: {abiertos} · integrados: {integrados} · reviews: {reviews}"
            )
        } else {
            let unidad = match tipo {
                "Tickets" => "cambios de estado",
                "Commits" => "commits",
                "Reuniones" => "reuniones",
                _ => "movimientos",
            };
            format!("{} {unidad}", items.len())
        };
        let mut repos = BTreeMap::<String, Vec<&Item>>::new();
        if matches!(tipo, "Pull requests" | "Commits") {
            for item in &items {
                let repo = url::Url::parse(&item.url)
                    .ok()
                    .and_then(|u| {
                        u.path_segments()
                            .map(|s| s.take(2).collect::<Vec<_>>().join("/"))
                    })
                    .unwrap_or_else(|| "Sin repositorio".into());
                repos.entry(repo).or_default().push(item);
            }
        }
        let repos = repos
            .into_iter()
            .map(|(repo, items)| {
                let resumen = if tipo == "Pull requests" {
                    let prs = items.iter().map(|i| &i.url).collect::<BTreeSet<_>>().len();
                    let unidad = if prs == 1 { "PR" } else { "PRs" };
                    format!("{prs} {unidad} · {} movimientos", items.len())
                } else {
                    format!("{} commits", items.len())
                };
                json!({"titulo":repo,"resumen":resumen,"items":items})
            })
            .collect::<Vec<_>>();
        Some(json!({"titulo":tipo,"resumen":resumen,"items":items,"repos":repos}))
    })
    .collect()
}

fn prs_agrupados<'a>(prs: impl Iterator<Item = &'a Pr>) -> Vec<Value> {
    let mut repos = BTreeMap::<&str, Vec<&Pr>>::new();
    for pr in prs {
        repos.entry(&pr.repo).or_default().push(pr);
    }
    repos
        .into_iter()
        .map(|(repo, prs)| {
            let bloqueados = prs
                .iter()
                .filter(|p| p.conflictos || matches!(p.ci.as_str(), "FAILURE" | "ERROR"))
                .count();
            let borradores = prs.iter().filter(|p| p.borrador).count();
            let unidad = if prs.len() == 1 { "PR" } else { "PRs" };
            let mut resumen = vec![format!("{} {unidad}", prs.len())];
            if bloqueados > 0 {
                resumen.push(format!("{bloqueados} con CI fallido o conflictos"));
            }
            if borradores > 0 {
                let unidad = if borradores == 1 {
                    "borrador"
                } else {
                    "borradores"
                };
                resumen.push(format!("{borradores} {unidad}"));
            }
            json!({"titulo":repo,"resumen":resumen.join(" · "),"prs":prs})
        })
        .collect()
}

fn por_persona(items: &[Value]) -> Vec<Value> {
    let mut personas = BTreeMap::<&str, Vec<&Value>>::new();
    for item in items {
        personas
            .entry(item["persona"].as_str().unwrap_or("Sin asignar"))
            .or_default()
            .push(item);
    }
    personas
        .into_iter()
        .map(|(persona, items)| json!({"persona":persona,"items":items}))
        .collect()
}

pub fn ejecutar(c: &Config, fecha: NaiveDate) -> Result<()> {
    let dir = archivos::dia(&c.salida, fecha);
    let d: Datos = archivos::leer(&dir.join("datos.json"))?;
    let e: Edicion = archivos::leer(&dir.join("edicion.json"))?;
    editar::validar(&e, &d)?;
    let refs = editar::referencias(&d);
    let top = e
        .top
        .iter()
        .map(|t| {
            let (_, url) = &refs[&t.referencia];
            json!({"titulo":t.titulo,"texto":t.texto,"url":url})
        })
        .collect::<Vec<_>>();
    let mut filas = vec![];
    if let Some(j) = &d.jira.datos {
        for t in &j.tickets {
            let prs = d
                .github
                .datos
                .as_ref()
                .map(|g| alertas::enlazados(t, &g.prs))
                .unwrap_or_default();
            let ultima = prs
                .iter()
                .filter_map(|p| p.ultimo_commit)
                .chain(std::iter::once(t.actualizado))
                .max()
                .unwrap();
            let ahora = if d.demo {
                fecha.and_hms_opt(15, 0, 0).unwrap().and_utc()
            } else {
                Utc::now()
            };
            let dias = ahora.signed_duration_since(ultima).num_days().max(0);
            let off = FixedOffset::west_opt(4 * 3600).unwrap();
            let cerrados = j
                .tickets
                .iter()
                .filter(|x| {
                    x.persona == t.persona
                        && x.cerrado
                            .is_some_and(|f| f.with_timezone(&off).date_naive() == d.ayer_habil)
                })
                .map(|x| x.clave.clone())
                .collect::<Vec<_>>();
            filas.push(json!({"persona":t.persona,"clave":t.clave,"titulo":t.titulo,"url":t.url,"estado":if t.categoria=="done"{"Listo"}else if t.categoria=="in_progress"{"En curso"}else{"Informativo"},"estado_jira":t.estado,"prs":prs,"dias":dias,"ayer":cerrados.join(", ")}));
        }
    }
    let prioridad = |s: &str| {
        c.personas_primero
            .iter()
            .position(|p| s.to_lowercase().contains(&p.to_lowercase()))
            .unwrap_or(c.personas_primero.len())
    };
    filas.sort_by(|a, b| {
        (
            prioridad(a["persona"].as_str().unwrap()),
            a["persona"].as_str().unwrap(),
            std::cmp::Reverse(a["dias"].as_i64().unwrap()),
        )
            .cmp(&(
                prioridad(b["persona"].as_str().unwrap()),
                b["persona"].as_str().unwrap(),
                std::cmp::Reverse(b["dias"].as_i64().unwrap()),
            ))
    });
    let mut agenda = vec![];
    let agenda_inicio = fecha.and_hms_opt(4, 0, 0).unwrap().and_utc();
    let agenda_fin = (fecha + Duration::days(2))
        .and_hms_opt(4, 0, 0)
        .unwrap()
        .and_utc();
    if let Some(cal) = &d.calendario.datos {
        for ev in cal
            .eventos
            .iter()
            .filter(|ev| ev.inicio < agenda_fin && ev.fin > agenda_inicio)
        {
            let choque = ev.ocupa_agenda
                && !ev.cancelado
                && cal.eventos.iter().any(|x| {
                    x.ocupa_agenda
                        && !x.cancelado
                        && x.item.id != ev.item.id
                        && x.inicio < ev.fin
                        && x.fin > ev.inicio
                });
            agenda.push(json!({"item":ev.item,"inicio":ev.inicio,"fin":ev.fin,"cancelado":ev.cancelado,"choque":choque}));
        }
    }
    agenda.sort_by_key(|a| a["inicio"].as_str().unwrap_or_default().to_owned());
    let slack = d
        .slack
        .datos
        .as_ref()
        .map(|s| alertas::slack_accion(s, c))
        .unwrap_or_default();
    let mut bots = BTreeMap::<String, usize>::new();
    if let Some(s) = &d.slack.datos {
        for m in &s.mensajes {
            let canal = m.canal.trim_start_matches('#');
            if c.canales_bot.iter().any(|b| b == canal) {
                *bots.entry(canal.into()).or_default() += 1;
            }
        }
    }
    let grupos = d
        .gmail
        .datos
        .as_ref()
        .map(alertas::honeybadger)
        .unwrap_or_default();
    let mut maquinas = BTreeMap::<String, usize>::new();
    if let Some(g) = &d.gmail.datos {
        for m in g
            .correos
            .iter()
            .filter(|m| m.clase != "persona" && m.tipo_maquina.as_deref() != Some("honeybadger"))
        {
            *maquinas
                .entry(
                    m.tipo_maquina
                        .clone()
                        .unwrap_or_else(|| "Otros avisos".into()),
                )
                .or_default() += 1;
        }
    }
    let conteos = |x: &Datos| -> BTreeMap<String, Option<i64>> {
        BTreeMap::from([
            (
                "correo".into(),
                x.gmail
                    .datos
                    .as_ref()
                    .map(|g| g.correos.iter().filter(|m| m.clase == "persona").count() as i64),
            ),
            (
                "slack".into(),
                x.slack
                    .datos
                    .as_ref()
                    .map(|s| alertas::slack_accion(s, c).len() as i64),
            ),
            (
                "prs".into(),
                x.github.datos.as_ref().map(|g| {
                    g.prs
                        .iter()
                        .filter(|p| p.abierto && p.autor == c.github_usuario)
                        .count() as i64
                }),
            ),
            (
                "equipo".into(),
                x.jira.datos.as_ref().map(|j| j.tickets.len() as i64),
            ),
            (
                "produccion".into(),
                x.gmail.datos.as_ref().map(|g| {
                    alertas::honeybadger(g)
                        .iter()
                        .map(|g| g.avisos as i64)
                        .sum()
                }),
            ),
        ])
    };
    let anterior: Option<Datos> =
        archivos::leer(&archivos::dia(&c.salida, fecha - Duration::days(1)).join("datos.json"))
            .ok();
    let hoy = conteos(&d);
    let ayer = anterior.as_ref().map(conteos).unwrap_or_default();
    let comparaciones = hoy
        .iter()
        .map(|(k, n)| {
            let text = match (n, ayer.get(k).copied().flatten()) {
                (Some(n), Some(a)) => format!("{:+} frente a ayer", n - a),
                _ => "Sin dato de ayer".into(),
            };
            (k.clone(), text)
        })
        .collect::<BTreeMap<_, _>>();
    let avisos: usize = grupos.iter().map(|g| g.avisos).sum();
    let pagos: usize = grupos.iter().map(|g| g.pagos).sum();
    let actividad = actividad_agrupada(&d);
    let mut mis_prs = vec![];
    let mut reviews = vec![];
    if let Some(g) = &d.github.datos {
        mis_prs = prs_agrupados(
            g.prs
                .iter()
                .filter(|p| p.abierto && p.autor == c.github_usuario),
        );
        reviews = prs_agrupados(
            g.prs
                .iter()
                .filter(|p| p.abierto && p.solicitado.contains(&c.github_usuario)),
        );
    }
    let mut equipo = por_persona(&filas);
    let mut alertas = por_persona(
        &serde_json::to_value(&d.alertas)?
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    for grupos in [&mut equipo, &mut alertas] {
        grupos.sort_by_key(|g| prioridad(g["persona"].as_str().unwrap_or_default()));
    }
    let mut vista = json!({"d":d,"e":e,"top":top,"filas":filas,"agenda":agenda,"slack":slack,"bots":bots,"grupos":grupos,"avisos":avisos,"pagos":pagos,"maquinas":maquinas,"comparaciones":comparaciones,"actividad":actividad,"mis_prs":mis_prs,"reviews":reviews,"equipo":equipo,"alertas":alertas});
    limpiar_urls(&mut vista);
    let mut env = entorno();
    env.add_template("diario", include_str!("../plantilla.html"))?;
    let html=env.get_template("diario")?.render(context!{v=>vista,c=>c.ciudad.clone(),usuario=>c.github_usuario.clone(),reporte_url=>c.reporte_url.clone(),audio=>c.salida.join(format!("{fecha}.m4a")).is_file()&&!dir.join("audio-pendiente.txt").is_file(),audio_pendiente=>dir.join("audio-pendiente.txt").is_file(),audio_error=>dir.join("audio-error.txt").is_file()})?;
    std::fs::create_dir_all(c.salida.join("estilos"))?;
    archivos::atomico(
        &c.salida.join("estilos/pico.min.css"),
        include_bytes!("../estilos/pico.min.css"),
    )?;
    archivos::atomico(
        &c.salida.join("estilos/tokens.css"),
        include_bytes!("../estilos/tokens.css"),
    )?;
    archivos::atomico(&c.salida.join("diario.js"), include_bytes!("../diario.js"))?;
    archivos::atomico(&c.salida.join(format!("{fecha}.html")), html.as_bytes())?;
    archivos::atomico(&c.salida.join("ultimo.html"), html.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enlaces_y_escape() {
        for s in [
            "javascript:alert(1)",
            "data:text/html,x",
            "//evil.test",
            "https://user:pass@a.com",
        ] {
            assert!(!url_segura(s));
        }
        assert!(url_segura("https://github.com/a/b/pull/1"));
        let mut e = entorno();
        e.add_template("x", "{{ texto }}").unwrap();
        assert_eq!(
            e.get_template("x")
                .unwrap()
                .render(context! {texto=>"<script>"})
                .unwrap(),
            "&lt;script&gt;"
        );
    }
}
