use super::claude;
use crate::{
    archivos,
    config::Config,
    modelo::{Estado, Fuente, Item, Jira, Sprint, Ticket},
};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
    time::Instant,
};

const BUSCAR: &str = "mcp__claude_ai_Atlassian__searchJiraIssuesUsingJql";
const ISSUE: &str = "mcp__claude_ai_Atlassian__getJiraIssue";

fn fecha(v: &Value) -> Option<DateTime<Utc>> {
    v.as_str()
        .and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .or_else(|_| DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z"))
                .ok()
        })
        .map(|f| f.with_timezone(&Utc))
}

fn clave(v: &Value) -> Result<&str> {
    let s = v["key"].as_str().context("Issue sin clave")?;
    anyhow::ensure!(
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "Clave Jira inválida"
    );
    Ok(s)
}

fn historia(v: &Value) -> Option<&[Value]> {
    v["changelog"]["histories"].as_array().map(Vec::as_slice)
}

fn historia_completa(v: &Value) -> bool {
    historia(v).is_some_and(|xs| {
        v["changelog"]["startAt"].as_u64() == Some(0)
            && v["changelog"]["total"].as_u64() == Some(xs.len() as u64)
            && xs
                .iter()
                .all(|h| fecha(&h["created"]).is_some() && h["items"].is_array())
    })
}

fn categorias(xs: &BTreeMap<String, Value>) -> BTreeMap<String, String> {
    xs.values()
        .filter_map(|v| {
            Some((
                v["fields"]["status"]["id"].as_str()?.into(),
                v["fields"]["status"]["statusCategory"]["key"]
                    .as_str()?
                    .into(),
            ))
        })
        .collect()
}

fn ticket(v: &Value, c: &Config, categorias: &BTreeMap<String, String>) -> Result<Ticket> {
    let key = clave(v)?;
    let f = &v["fields"];
    let actualizado = fecha(&f["updated"]).context("Issue sin fecha updated original")?;
    let persona = f["assignee"]["displayName"]
        .as_str()
        .unwrap_or("Sin asignar")
        .to_owned();
    let categoria = match f["status"]["statusCategory"]["key"].as_str() {
        Some("done") => "done",
        Some("indeterminate") => "in_progress",
        Some("new") => "todo",
        _ => anyhow::bail!("Issue sin categoría de estado verificable"),
    };
    let url = format!("{}/browse/{key}", c.jira_sitio.trim_end_matches('/'));
    let mut cambios = vec![];
    let mut inicios = vec![];
    if let Some(hs) = historia(v) {
        for h in hs {
            let Some(cuando) = fecha(&h["created"]) else {
                continue;
            };
            for (i, x) in h["items"].as_array().into_iter().flatten().enumerate() {
                if x["field"] != "status" {
                    continue;
                }
                if x["to"]
                    .as_str()
                    .and_then(|s| categorias.get(s))
                    .is_some_and(|s| s == "indeterminate")
                {
                    inicios.push(cuando);
                }
                cambios.push(Item {
                    id: format!("jira-{key}-{}-{i}", h["id"].as_str().unwrap_or("cambio")),
                    titulo: format!(
                        "{key} pasó a {}",
                        x["toString"].as_str().unwrap_or("otro estado")
                    ),
                    detalle: format!(
                        "{} → {}",
                        x["fromString"].as_str().unwrap_or(""),
                        x["toString"].as_str().unwrap_or("")
                    ),
                    url: url.clone(),
                    persona: persona.clone(),
                    estado: if categoria == "done" {
                        Estado::Listo
                    } else {
                        Estado::Informativo
                    },
                    fecha: cuando,
                    sospechoso: false,
                });
            }
        }
    }
    Ok(Ticket {
        clave: key.into(),
        titulo: f["summary"]
            .as_str()
            .context("Issue sin summary original")?
            .into(),
        url,
        github: c
            .identidades
            .iter()
            .find(|(p, _)| p.eq_ignore_ascii_case(&persona))
            .map(|(_, login)| login.clone()),
        persona,
        estado: f["status"]["name"]
            .as_str()
            .context("Issue sin estado original")?
            .into(),
        categoria: categoria.into(),
        actualizado,
        iniciado: historia_completa(v)
            .then(|| inicios.into_iter().min())
            .flatten(),
        cerrado: if categoria == "done" {
            fecha(&f["resolutiondate"])
        } else {
            None
        },
        prs: vec![],
        cambios,
    })
}

fn ids(v: &Value) -> Option<BTreeSet<String>> {
    if v.is_null() {
        return Some(BTreeSet::new());
    }
    let s = v.as_str()?;
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.bytes().all(|b| b.is_ascii_digit()).then(|| s.to_owned()))
        .collect()
}

fn sprints(
    xs: &BTreeMap<String, Value>,
    categorias: &BTreeMap<String, String>,
    campo: &str,
    tablero: u64,
    dia: NaiveDate,
) -> Vec<Sprint> {
    let mut metadatos = BTreeMap::new();
    for v in xs.values() {
        for s in v["fields"][campo].as_array().into_iter().flatten() {
            if s["originBoardId"]
                .as_u64()
                .or_else(|| s["boardId"].as_u64())
                == Some(tablero)
            {
                if let Some(id) = s["id"].as_u64() {
                    metadatos.insert(id.to_string(), s.clone());
                }
            }
        }
    }
    let off = FixedOffset::west_opt(4 * 3600).unwrap();
    let mut salida = vec![];
    for (id, s) in metadatos {
        let Some(inicio) = fecha(&s["startDate"]) else {
            continue;
        };
        let Some(fin) = fecha(&s["endDate"]) else {
            continue;
        };
        if fin.with_timezone(&off).date_naive() < dia - Duration::days(62) {
            continue;
        }
        let cerrado = s["state"] == "closed";
        let corte = if cerrado {
            fecha(&s["completeDate"]).unwrap_or(fin)
        } else {
            Utc::now()
        };
        let mut completos = true;
        let mut comprometidos = vec![];
        let mut terminados = vec![];
        let mut arrastrados = vec![];
        for (key, v) in xs {
            let Some(creado) = fecha(&v["fields"]["created"]) else {
                completos = false;
                continue;
            };
            if creado > inicio {
                continue;
            }
            if !historia_completa(v) {
                completos = false;
                continue;
            }
            let mut miembros = v["fields"][campo]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["id"].as_u64().map(|i| i.to_string()))
                .collect::<BTreeSet<_>>();
            let mut estado = v["fields"]["status"]["id"].as_str().map(str::to_owned);
            let mut hs = historia(v).unwrap().iter().collect::<Vec<_>>();
            hs.sort_by_key(|h| std::cmp::Reverse(fecha(&h["created"])));
            for h in hs {
                let Some(cuando) = fecha(&h["created"]) else {
                    completos = false;
                    continue;
                };
                for x in h["items"].as_array().into_iter().flatten() {
                    if cuando > inicio
                        && (x["fieldId"] == campo
                            || x["field"]
                                .as_str()
                                .is_some_and(|s| s.eq_ignore_ascii_case("Sprint")))
                    {
                        if let Some(anteriores) = ids(&x["from"]) {
                            miembros = anteriores;
                        } else {
                            completos = false;
                        }
                    }
                    if cuando > corte && x["field"] == "status" {
                        estado = x["from"].as_str().map(str::to_owned);
                    }
                }
            }
            if miembros.contains(&id) {
                comprometidos.push(key.clone());
                match estado
                    .as_deref()
                    .and_then(|s| categorias.get(s))
                    .map(String::as_str)
                {
                    Some("done") => terminados.push(key.clone()),
                    Some("new" | "indeterminate") => arrastrados.push(key.clone()),
                    _ => completos = false,
                }
            }
        }
        salida.push(Sprint { id, nombre:s["name"].as_str().unwrap_or("Sprint sin nombre").into(), inicio:inicio.with_timezone(&off).date_naive(), fin:if cerrado {corte} else {fin}.with_timezone(&off).date_naive(), comprometidos, terminados, arrastrados, completo:completos, estado:if cerrado {"closed"} else {"active"}.into(), evidencia:if completos {"Pertenencia al inicio y estado al cierre reconstruidos por Rust desde el changelog completo. Terminados y pendientes corresponden al compromiso inicial."} else {"No disponible: falta historial completo o una categoría verificable de un estado histórico para reconstruir el compromiso y su cierre; los conjuntos parciales no son totales."}.into() });
    }
    salida.sort_by_key(|s| s.inicio);
    salida
}

fn detalle(c: &Config, key: &str, campo: Option<&str>) -> Result<Value> {
    let mut fields = vec![
        "summary",
        "status",
        "assignee",
        "created",
        "updated",
        "resolutiondate",
    ];
    if let Some(campo) = campo {
        fields.push(campo);
    } else {
        fields = vec!["*all"];
    }
    claude::lectura_original(
        c,
        ISSUE,
        &json!({"cloudId":c.jira_sitio,"issueIdOrKey":key,"fields":fields,"expand":if campo.is_some(){"changelog"}else{"names,changelog"},"updateHistory":false,"responseContentFormat":"adf"}),
        "fields",
    )
}

fn cache(root: &Path, key: &str, updated: &Value) -> Option<Value> {
    let v: Value = archivos::leer(&root.join(format!("{key}.json"))).ok()?;
    (v["fields"]["updated"] == *updated && v["key"] == key).then_some(v)
}

pub fn obtener(c: &Config, dia: NaiveDate) -> Result<Fuente<Jira>> {
    let inicio = Instant::now();
    let filtro = c
        .jira_filtro
        .context("Falta el id verificado del filtro del tablero Jira")?;
    let root = c.salida.join("jira");
    let historicos = root.join("historial");
    let meta_path = root.join(format!("tablero-{}.json", c.jira_tablero));
    let meta: Option<Value> = archivos::leer(&meta_path).ok();
    let mut campo = meta
        .as_ref()
        .and_then(|v| v["sprint_field"].as_str())
        .map(str::to_owned);
    if campo.is_none() {
        let muestra = claude::lectura_original(
            c,
            BUSCAR,
            &json!({"cloudId":c.jira_sitio,"jql":format!("filter = {filtro} ORDER BY updated DESC, key ASC"),"fields":["summary","updated"],"maxResults":1,"searchResultMode":"issues","responseContentFormat":"adf"}),
            "issues",
        )?;
        if let Some(v) = muestra["issues"]["nodes"]
            .as_array()
            .and_then(|a| a.first())
        {
            let key = clave(v)?;
            let d = detalle(c, key, None)?;
            campo = d["names"]
                .as_object()
                .and_then(|m| {
                    m.iter()
                        .find(|(_, v)| v.as_str().is_some_and(|s| s.eq_ignore_ascii_case("Sprint")))
                })
                .map(|(k, _)| k.clone());
            archivos::guardar(&historicos.join(format!("{key}.json")), &d)?;
            if let Some(campo) = &campo {
                archivos::guardar(
                    &meta_path,
                    &json!({"sprint_field":campo,"actualizado":Utc::now(),"filtro":filtro}),
                )?;
            }
        }
    }
    let mut xs = BTreeMap::new();
    let mut cursor = None::<String>;
    let mut vistos = BTreeSet::new();
    let mut completo = false;
    let mut errores = vec![];
    let mut paginas = 0;
    for pagina in 1..=100 {
        let mut fields = vec![
            "summary",
            "status",
            "assignee",
            "created",
            "updated",
            "resolutiondate",
        ];
        if let Some(campo) = &campo {
            fields.push(campo);
        }
        let mut entrada = json!({"cloudId":c.jira_sitio,"jql":format!("filter = {filtro} ORDER BY updated DESC, key ASC"),"fields":fields,"maxResults":100,"searchResultMode":"issues","responseContentFormat":"adf"});
        if let Some(cursor) = &cursor {
            entrada["nextPageToken"] = json!(cursor);
        }
        let p = match claude::lectura_original(c, BUSCAR, &entrada, "issues") {
            Ok(v) => v,
            Err(e) => {
                errores.push(format!("Página {pagina}: {e:#}"));
                break;
            }
        };
        archivos::guardar(
            &root
                .join(dia.to_string())
                .join(format!("pagina-{pagina}.json")),
            &p,
        )?;
        paginas += 1;
        for v in p["issues"]["nodes"]
            .as_array()
            .context("Respuesta Jira sin issues.nodes")?
        {
            let key = clave(v)?.to_owned();
            if xs.insert(key, v.clone()).is_some() {
                errores.push(
                    "Jira repitió un ticket entre páginas; no se garantiza cobertura estable"
                        .into(),
                );
            }
        }
        match p["issues"]["pageInfo"]["hasNextPage"].as_bool() {
            Some(false) => {
                completo = true;
                break;
            }
            Some(true) => {
                let next = p["issues"]["pageInfo"]["endCursor"]
                    .as_str()
                    .context("Jira anuncia otra página sin cursor")?
                    .to_owned();
                anyhow::ensure!(
                    !next.is_empty() && vistos.insert(next.clone()),
                    "Cursor Jira vacío o repetido"
                );
                cursor = Some(next);
            }
            None => anyhow::bail!("Jira no informa si quedan páginas; no se supone que terminó"),
        }
        if inicio.elapsed().as_secs() >= c.timeout_segundos {
            break;
        }
    }
    if !completo {
        errores.push("La paginación del filtro no terminó".into());
    }
    let mut pendientes = vec![];
    for (key, v) in &mut xs {
        if let Some(d) = cache(&historicos, key, &v["fields"]["updated"]) {
            *v = d;
        } else {
            pendientes.push(key.clone());
        }
    }
    let indice = AtomicUsize::new(0);
    let resultados = Mutex::new(vec![]);
    std::thread::scope(|scope| {
        for _ in 0..4.min(pendientes.len()) {
            scope.spawn(||loop {
                let restante=c.timeout_segundos.saturating_sub(inicio.elapsed().as_secs());
                if restante<10 {break;}
                let i=indice.fetch_add(8,Ordering::Relaxed);
                if i>=pendientes.len() {break;}
                let keys=&pendientes[i..(i+8).min(pendientes.len())];
                let mut config=c.clone();config.timeout_segundos=restante.min(120);
                let mut fields=vec!["summary","status","assignee","created","updated","resolutiondate"];
                if let Some(campo)=campo.as_deref() {fields.push(campo);}
                let entradas=keys.iter().map(|key|json!({"cloudId":c.jira_sitio,"issueIdOrKey":key,"fields":fields,"expand":"changelog","updateHistory":false,"responseContentFormat":"adf"})).collect::<Vec<_>>();
                match claude::lecturas_originales(&config,ISSUE,&entradas,"fields") {
                    Ok(ds)=>{
                        let mut ds=ds.into_iter().filter_map(|v|v["key"].as_str().map(str::to_owned).map(|k|(k,v))).collect::<BTreeMap<_,_>>();
                        for key in keys {
                            let r=ds.remove(key).context("La lectura no devolvió este ticket").and_then(|d|{
                                anyhow::ensure!(fecha(&d["fields"]["updated"]).is_some(),"Detalle sin updated original");
                                archivos::guardar(&historicos.join(format!("{key}.json")),&d)?;Ok(d)
                            });
                            resultados.lock().unwrap().push((key.clone(),r));
                        }
                    },
                    Err(e)=>{let error=format!("{e:#}");for key in keys {resultados.lock().unwrap().push((key.clone(),Err(anyhow::anyhow!(error.clone()))));}},
                }
            });
        }
    });
    for (key, r) in resultados.into_inner().unwrap() {
        match r {
            Ok(v) => {
                xs.insert(key, v);
            }
            Err(e) => errores.push(format!("Historial {key}: {e:#}")),
        }
    }
    let mut categorias = categorias(&xs);
    let faltantes = xs
        .values()
        .flat_map(|v| historia(v).into_iter().flatten())
        .flat_map(|h| h["items"].as_array().into_iter().flatten())
        .filter(|x| x["field"] == "status")
        .flat_map(|x| [x["from"].as_str(), x["to"].as_str()])
        .flatten()
        .filter(|s| {
            !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && !categorias.contains_key(*s)
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if !faltantes.is_empty() {
        let entradas = faltantes.iter().map(|id| json!({"cloudId":c.jira_sitio,"jql":format!("project = {} AND status = {id}", c.jira_proyecto),"fields":["status"],"maxResults":1,"searchResultMode":"issues","responseContentFormat":"adf"})).collect::<Vec<_>>();
        if let Ok(paginas) = claude::lecturas_originales(c, BUSCAR, &entradas, "issues") {
            for pagina in paginas {
                for v in pagina["issues"]["nodes"].as_array().into_iter().flatten() {
                    if let (Some(id), Some(cat)) = (
                        v["fields"]["status"]["id"].as_str(),
                        v["fields"]["status"]["statusCategory"]["key"].as_str(),
                    ) {
                        categorias.insert(id.into(), cat.into());
                    }
                }
            }
        }
    }
    let mut tickets = vec![];
    let mut personas = c.jira_personas.iter().cloned().collect::<BTreeSet<_>>();
    for (key, v) in &xs {
        match ticket(v, c, &categorias) {
            Ok(t) => {
                personas.insert(t.persona.clone());
                tickets.push(t);
            }
            Err(e) => errores.push(format!("Ticket {key}: {e:#}")),
        }
    }
    let historiales = xs.values().filter(|v| historia_completa(v)).count();
    if historiales < xs.len() {
        errores.push(format!("Tickets con historial completo: {historiales}/{}. Se conservan las fechas originales; el inicio y el compromiso de sprint desconocidos no se estiman",xs.len()));
    }
    if campo.is_none() {
        errores
            .push("No se identificó el campo Sprint en names; no se supone un customfield".into());
    }
    let sprints = campo
        .as_deref()
        .map(|campo| sprints(&xs, &categorias, campo, c.jira_tablero, dia))
        .unwrap_or_default();
    let mut f=Fuente::ok(Jira {tickets,sprints,personas:personas.into_iter().collect()},vec![format!("Filtro vivo {filtro}, tablero {}; sin filtro por persona. Configuración verificada en Jira UI: {}/jira/software/c/projects/{}/boards/{}/settings/filter",c.jira_tablero,c.jira_sitio,c.jira_proyecto,c.jira_tablero),format!("{paginas} páginas; {} tickets originales; {historiales} historiales completos. Paginación y fechas extraídas por Rust de respuestas MCP originales",xs.len()),"Solo búsqueda JQL y getJiraIssue en la conexión autorizada a paybycourt. Parámetros exactos protegidos por hook; updateHistory=false. Historial sin cambios reutilizado desde archivos locales".into()]);
    f.completo = completo && errores.is_empty();
    if !errores.is_empty() {
        f.error = Some(errores.into_iter().take(12).collect::<Vec<_>>().join("; "));
    }
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fechas_jira_conservan_el_instante_y_no_se_estiman() {
        assert_eq!(
            fecha(&json!("2026-11-19T14:05:00.000-0400"))
                .unwrap()
                .to_rfc3339(),
            "2026-11-19T18:05:00+00:00"
        );
        assert!(fecha(&Value::Null).is_none());
        assert!(fecha(&json!("fecha aproximada")).is_none());
        assert!(!historia_completa(
            &json!({"changelog":{"startAt":0,"total":2,"histories":[]}})
        ));
    }

    #[test]
    fn sprint_reconstruye_retirados_y_reaperturas_despues_del_cierre() {
        let sprint = json!({"id":7,"originBoardId":590,"name":"Sprint de prueba","state":"closed","startDate":"2026-11-01T04:00:00Z","endDate":"2026-11-07T04:00:00Z","completeDate":"2026-11-07T12:00:00Z"});
        let a = json!({"key":"CORE-1","fields":{"created":"2026-10-01T04:00:00Z","status":{"id":"2","statusCategory":{"key":"indeterminate"}},"sprint":[]},"changelog":{"startAt":0,"total":2,"histories":[{"created":"2026-11-02T12:00:00Z","items":[{"field":"Sprint","fieldId":"sprint","from":"7","to":""}]},{"created":"2026-11-08T12:00:00Z","items":[{"field":"status","from":"3","to":"2"}]}]}});
        let b = json!({"key":"CORE-2","fields":{"created":"2026-10-01T04:00:00Z","status":{"id":"3","statusCategory":{"key":"done"}},"sprint":[sprint]},"changelog":{"startAt":0,"total":0,"histories":[]}});
        let mut xs = BTreeMap::from([("CORE-1".into(), a), ("CORE-2".into(), b)]);
        let dia = NaiveDate::from_ymd_opt(2026, 11, 19).unwrap();
        let ss = sprints(&xs, &categorias(&xs), "sprint", 590, dia);
        assert_eq!(ss.len(), 1);
        assert!(ss[0].completo);
        assert_eq!(ss[0].comprometidos, vec!["CORE-1", "CORE-2"]);
        assert_eq!(ss[0].terminados, vec!["CORE-1", "CORE-2"]);
        assert!(ss[0].arrastrados.is_empty());
        xs.get_mut("CORE-1").unwrap()["changelog"]["histories"][1]["items"][0]["from"] = json!("4");
        let mut cats = categorias(&xs);
        assert!(!sprints(&xs, &cats, "sprint", 590, dia)[0].completo);
        cats.insert("4".into(), "indeterminate".into());
        let ss = sprints(&xs, &cats, "sprint", 590, dia);
        assert!(ss[0].completo);
        assert_eq!(ss[0].arrastrados, vec!["CORE-1"]);
        xs.get_mut("CORE-1").unwrap()["changelog"]["total"] = json!(3);
        assert!(!sprints(&xs, &categorias(&xs), "sprint", 590, dia)[0].completo);
    }
}
