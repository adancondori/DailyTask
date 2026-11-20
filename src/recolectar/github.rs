use crate::{config::Config, modelo::*, proceso};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const QUERY: &str = r#"query($q:String!,$cursor:String,$detalles:Boolean!,$tamano:Int!){search(query:$q,type:ISSUE,first:$tamano,after:$cursor){issueCount pageInfo{hasNextPage endCursor} nodes{... on PullRequest{number title body url author{login __typename} repository{nameWithOwner} createdAt updatedAt mergedAt isDraft state mergeable headRefName headRefOid commits(last:1) @include(if:$detalles){nodes{commit{committedDate statusCheckRollup{state}}}} reviews(first:100) @include(if:$detalles){totalCount nodes{author{login __typename} submittedAt state}} reviewRequests(first:100) @include(if:$detalles){totalCount nodes{requestedReviewer{... on User{login} ... on Team{slug}}}}}}}}"#;

fn gh(args: Vec<String>, c: &Config) -> Result<Value> {
    let mut ultimo = None;
    for intento in 0..3 {
        match proceso::ejecutar("gh", &args, "", c.timeout_segundos)
            .and_then(|s| serde_json::from_str(&s).context("gh devolvió JSON inválido"))
        {
            Ok(v) => return Ok(v),
            Err(e) => ultimo = Some(e),
        }
        if intento < 2 {
            std::thread::sleep(std::time::Duration::from_secs(1 << intento));
        }
    }
    Err(ultimo.unwrap())
}
fn date(v: &Value) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v.as_str().context("Falta fecha GitHub")?)
        .map(|d| d.with_timezone(&Utc))
        .context("Fecha GitHub inválida")
}
fn strv(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_owned()
}
fn nodos_pr(c: &Config, repo: &str, numero: u64, campo: &str) -> Result<Vec<Value>> {
    let (owner, repo) = repo
        .split_once('/')
        .context("Repositorio sin organización")?;
    let seleccion=match campo{
        "reviews"=>"reviews(first:100,after:$cursor){pageInfo{hasNextPage endCursor} nodes{author{login __typename} submittedAt state}}",
        "reviewRequests"=>"reviewRequests(first:100,after:$cursor){pageInfo{hasNextPage endCursor} nodes{requestedReviewer{... on User{login} ... on Team{slug}}}}",
        _=>bail!("Conexión PR no soportada"),
    };
    let query="query($owner:String!,$repo:String!,$numero:Int!,$cursor:String){repository(owner:$owner,name:$repo){pullRequest(number:$numero){CONEXION}}}".replace("CONEXION",seleccion);
    let mut cursor = None;
    let mut out = vec![];
    loop {
        let mut args = vec![
            "api".into(),
            "graphql".into(),
            "-f".into(),
            format!("query={query}"),
            "-f".into(),
            format!("owner={owner}"),
            "-f".into(),
            format!("repo={repo}"),
            "-F".into(),
            format!("numero={numero}"),
        ];
        if let Some(cur) = &cursor {
            args.extend(["-f".into(), format!("cursor={cur}")]);
        }
        let v = gh(args, c)?;
        let page = &v["data"]["repository"]["pullRequest"][campo];
        out.extend(
            page["nodes"]
                .as_array()
                .context("Faltan nodos de reviews")?
                .iter()
                .cloned(),
        );
        if page["pageInfo"]["hasNextPage"] != true {
            break;
        }
        let next = page["pageInfo"]["endCursor"]
            .as_str()
            .context("Falta cursor de reviews")?
            .to_owned();
        if cursor.as_ref() == Some(&next) {
            bail!("Cursor de reviews repetido");
        }
        cursor = Some(next);
    }
    Ok(out)
}
fn commits(c: &Config, consultas: &[(String, String)]) -> Result<Vec<(String, Result<Value>)>> {
    std::thread::scope(|scope| {
        let handles = (0..4)
            .map(|i| {
                scope.spawn(move || {
                    consultas
                        .iter()
                        .skip(i)
                        .step_by(4)
                        .map(|(repo, url)| {
                            (
                                repo.clone(),
                                gh(
                                    vec![
                                        "api".into(),
                                        url.clone(),
                                        "--paginate".into(),
                                        "--slurp".into(),
                                    ],
                                    c,
                                ),
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        let mut out = vec![];
        for h in handles {
            out.extend(
                h.join()
                    .map_err(|_| anyhow::anyhow!("Recolección de commits interrumpida"))?,
            );
        }
        Ok(out)
    })
}

pub fn obtener(c: &Config, fecha: NaiveDate, ayer: NaiveDate) -> Result<Github> {
    let inicio = fecha - Duration::days(62);
    let hasta_actual = fecha + Duration::days(1);
    let origen = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    let mut qs = VecDeque::from([
        (
            format!("org:{} is:pr", c.github_organizacion),
            inicio,
            hasta_actual,
            "updated",
            false,
        ),
        (
            format!("author:{} is:pr", c.github_usuario),
            inicio,
            hasta_actual,
            "updated",
            false,
        ),
        (
            format!("review-requested:{} is:pr is:open", c.github_usuario),
            origen,
            hasta_actual,
            "created",
            true,
        ),
        (
            format!("org:{} is:pr is:open", c.github_organizacion),
            origen,
            hasta_actual,
            "created",
            true,
        ),
        (
            format!("org:{} is:pr", c.github_organizacion),
            ayer,
            hasta_actual,
            "updated",
            true,
        ),
    ]);
    let mut prs = BTreeMap::new();
    let mut ramas = BTreeSet::new();
    let mut advertencias = vec![];
    let re = Regex::new(r"\bCORE-\d+\b")?;
    while let Some((base, desde, hasta, campo, detalles)) = qs.pop_front() {
        let q = format!("{base} {campo}:{desde}..{hasta}");
        let mut cursor = None;
        loop {
            let mut args = vec![
                "api".into(),
                "graphql".into(),
                "-f".into(),
                format!("query={QUERY}"),
                "-f".into(),
                format!("q={q}"),
                "-F".into(),
                format!("detalles={detalles}"),
                "-F".into(),
                format!("tamano={}", if detalles { 25 } else { 100 }),
            ];
            if let Some(cur) = &cursor {
                args.extend(["-f".into(), format!("cursor={cur}")]);
            }
            let v = gh(args, c).with_context(|| {
                format!(
                    "Búsqueda GitHub: {q}; página posterior: {}",
                    cursor.is_some()
                )
            })?;
            if v.get("errors").is_some() {
                bail!("GitHub GraphQL reportó errores");
            }
            let s = &v["data"]["search"];
            if s["issueCount"].as_u64().unwrap_or(0) > 1000 {
                if desde < hasta {
                    let mitad =
                        desde + Duration::days(hasta.signed_duration_since(desde).num_days() / 2);
                    qs.push_front((
                        base.clone(),
                        mitad + Duration::days(1),
                        hasta,
                        campo,
                        detalles,
                    ));
                    qs.push_front((base.clone(), desde, mitad, campo, detalles));
                } else if !base.contains("repo:") && base.contains("org:") {
                    let repos = gh(
                        vec![
                            "api".into(),
                            format!("orgs/{}/repos?per_page=100&type=all", c.github_organizacion),
                            "--paginate".into(),
                            "--slurp".into(),
                        ],
                        c,
                    )?;
                    for page in repos.as_array().context("Faltan páginas de repositorios")? {
                        for repo in page.as_array().context("Faltan repositorios")? {
                            let nombre = repo["full_name"].as_str().context("Repo sin nombre")?;
                            qs.push_back((
                                format!("{base} repo:{nombre}"),
                                desde,
                                hasta,
                                campo,
                                detalles,
                            ));
                        }
                    }
                } else {
                    bail!(
                        "Más de 1000 PRs en un día/repositorio; no se puede garantizar cobertura"
                    );
                }
                break;
            }
            for n in s["nodes"].as_array().context("Faltan PRs GitHub")? {
                if n.is_null() {
                    continue;
                }
                let repo = strv(&n["repository"]["nameWithOwner"]);
                let numero = n["number"].as_u64().context("Falta número PR")?;
                let reviews_nodos = if n["reviews"]["totalCount"].as_u64().unwrap_or(0) > 100 {
                    nodos_pr(c, &repo, numero, "reviews")?
                } else {
                    n["reviews"]["nodes"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                };
                let solicitudes_nodos =
                    if n["reviewRequests"]["totalCount"].as_u64().unwrap_or(0) > 100 {
                        nodos_pr(c, &repo, numero, "reviewRequests")?
                    } else {
                        n["reviewRequests"]["nodes"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default()
                    };
                let reviews = reviews_nodos
                    .iter()
                    .filter(|r| !r["submittedAt"].is_null())
                    .map(|r| {
                        Ok(Review {
                            autor: strv(&r["author"]["login"]),
                            fecha: date(&r["submittedAt"])?,
                            estado: strv(&r["state"]),
                            bot: r["author"]["__typename"] == "Bot"
                                || strv(&r["author"]["login"]).ends_with("[bot]"),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                let solicitado = solicitudes_nodos
                    .iter()
                    .map(|r| strv(&r["requestedReviewer"]["login"]))
                    .filter(|s| !s.is_empty())
                    .collect();
                let texto = format!("{} {} {}", n["title"], n["body"], n["headRefName"]);
                let tickets = re
                    .find_iter(&texto)
                    .map(|m| m.as_str().to_owned())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                let ultimo = &n["commits"]["nodes"][0]["commit"];
                let p = Pr {
                    numero,
                    repo: repo.clone(),
                    titulo: strv(&n["title"]),
                    url: strv(&n["url"]),
                    autor: strv(&n["author"]["login"]),
                    creado: date(&n["createdAt"])?,
                    actualizado: date(&n["updatedAt"])?,
                    ultimo_commit: if ultimo.is_null() {
                        None
                    } else {
                        Some(date(&ultimo["committedDate"])?)
                    },
                    mergeado: if n["mergedAt"].is_null() {
                        None
                    } else {
                        Some(date(&n["mergedAt"])?)
                    },
                    reviews,
                    solicitado,
                    ci: if !detalles {
                        "No consultado (PR cerrado)".into()
                    } else if ultimo["statusCheckRollup"]["state"].is_null() {
                        "Sin checks".into()
                    } else {
                        strv(&ultimo["statusCheckRollup"]["state"])
                    },
                    conflictos: n["mergeable"] == "CONFLICTING",
                    borrador: n["isDraft"] == true,
                    abierto: n["state"] == "OPEN",
                    tickets,
                };
                if p.abierto
                    && p.actualizado
                        .with_timezone(&FixedOffset::west_opt(4 * 3600).unwrap())
                        .date_naive()
                        >= ayer
                {
                    if let Some(sha) = n["headRefOid"]
                        .as_str()
                        .filter(|s| s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit()))
                    {
                        ramas.insert((repo, sha.to_string()));
                    }
                }
                prs.insert(p.url.clone(), p);
            }
            let _ = super::progreso(
                &c.salida,
                "recolectar",
                Some((
                    "GitHub",
                    &format!("En curso: {} PRs consultados", prs.len()),
                )),
                true,
                None,
            );
            if s["pageInfo"]["hasNextPage"] != true {
                break;
            }
            let next = s["pageInfo"]["endCursor"]
                .as_str()
                .context("Falta cursor GitHub")?
                .to_string();
            if cursor.as_ref() == Some(&next) {
                bail!("Cursor GitHub repetido");
            }
            cursor = Some(next);
        }
    }
    let offset = FixedOffset::west_opt(4 * 3600).unwrap();
    let mut actividad = vec![];
    for p in prs.values() {
        for (f, tipo) in [(Some(p.creado), "PR abierto"), (p.mergeado, "PR mergeado")] {
            if let Some(f) = f.filter(|d| d.with_timezone(&offset).date_naive() == ayer) {
                actividad.push(Item {
                    id: format!("{}-{tipo}", p.url),
                    titulo: format!("{tipo}: {}", p.titulo),
                    detalle: p.repo.clone(),
                    url: p.url.clone(),
                    persona: p.autor.clone(),
                    estado: Estado::Listo,
                    fecha: f,
                    sospechoso: false,
                });
            }
        }
        for r in &p.reviews {
            if r.fecha.with_timezone(&offset).date_naive() == ayer {
                actividad.push(Item {
                    id: format!("{}-review-{}", p.url, r.autor),
                    titulo: format!("Review: {}", p.titulo),
                    detalle: r.estado.clone(),
                    url: p.url.clone(),
                    persona: r.autor.clone(),
                    estado: Estado::Listo,
                    fecha: r.fecha,
                    sospechoso: false,
                });
            }
        }
    }
    let since = format!("{ayer}T00:00:00-04:00");
    let until = format!("{}T00:00:00-04:00", ayer + Duration::days(1));
    let mut repos: BTreeSet<String> = c.repositorios.iter().cloned().collect();
    repos.extend(prs.values().map(|p| p.repo.clone()));
    let mut consultas = repos
        .into_iter()
        .map(|repo| {
            let endpoint = format!("repos/{repo}/commits?since={since}&until={until}&per_page=100");
            (repo, endpoint)
        })
        .collect::<Vec<_>>();
    consultas.extend(ramas.into_iter().map(|(repo, sha)| {
        let endpoint =
            format!("repos/{repo}/commits?sha={sha}&since={since}&until={until}&per_page=100");
        (repo, endpoint)
    }));
    for (repo, resultado) in commits(c, &consultas)? {
        match resultado {
            Ok(pages) => {
                for page in pages.as_array().context("Faltan páginas de commits")? {
                    for a in page.as_array().context("Faltan commits")? {
                        actividad.push(Item {
                            id: format!("commit-{}", strv(&a["sha"])),
                            titulo: strv(&a["commit"]["message"])
                                .lines()
                                .next()
                                .unwrap_or("")
                                .into(),
                            detalle: format!("Commit en {repo}"),
                            url: strv(&a["html_url"]),
                            persona: a["author"]["login"]
                                .as_str()
                                .unwrap_or(
                                    a["commit"]["author"]["name"]
                                        .as_str()
                                        .unwrap_or("Sin autor"),
                                )
                                .into(),
                            estado: Estado::Listo,
                            fecha: date(&a["commit"]["committer"]["date"])?,
                            sospechoso: false,
                        });
                    }
                }
            }
            Err(_) => advertencias.push(format!("No se pudieron leer commits de {repo}")),
        }
    }
    let mut vistos = BTreeSet::new();
    actividad.retain(|a| vistos.insert(a.id.clone()));
    advertencias.sort();
    advertencias.dedup();
    Ok(Github {
        prs: prs.into_values().collect(),
        actividad,
        completo: advertencias.is_empty(),
        advertencias,
    })
}
