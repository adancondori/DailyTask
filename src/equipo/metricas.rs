use super::{Fallos, Snapshot, Tabla};
use crate::modelo::Fuente;
use chrono::{Duration, NaiveDate};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::{collections::BTreeMap, str::FromStr};

fn campo<'a>(t: &'a Tabla, fila: &'a [Value], nombre: &str) -> Option<&'a Value> {
    fila.get(t.columnas.iter().position(|c| c == nombre)?)
}

fn numero(v: Option<&Value>) -> Option<Decimal> {
    v.and_then(|v| {
        Decimal::from_str(
            &v.as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| v.to_string()),
        )
        .ok()
    })
}

fn valida(t: &Tabla, campos: &[&str]) -> bool {
    t.columnas.iter().any(|c| c == "gateway")
        && campos.iter().all(|c| t.columnas.iter().any(|k| k == c))
        && t.filas.iter().all(|fila| {
            campo(t, fila, "gateway").is_some_and(Value::is_string)
                && campos.iter().all(|c| numero(campo(t, fila, c)).is_some())
        })
}

pub fn salud(h: &[Snapshot], inicio: NaiveDate, fin: NaiveDate) -> Value {
    let dias = (fin - inicio).num_days() + 1;
    let anterior = inicio - Duration::days(dias);
    let mut valores: BTreeMap<(String, String), [Decimal; 6]> = BTreeMap::new();
    let mut renovaciones: BTreeMap<String, [Decimal; 2]> = BTreeMap::new();
    let mut cobertura = [0; 2];
    let mut cobertura_renovaciones = [0; 2];
    for s in h.iter().filter(|s| s.fecha >= anterior && s.fecha <= fin) {
        let periodo = usize::from(s.fecha < inicio);
        if s.salud.completo {
            if let Some(t) = s.salud.datos.as_ref().filter(|t| {
                valida(
                    t,
                    &[
                        "volumen_capturado",
                        "capturados",
                        "autorizados_sin_capturar",
                    ],
                )
            }) {
                cobertura[periodo] += 1;
                for fila in &t.filas {
                    let Some(gateway) = campo(t, fila, "gateway").and_then(Value::as_str) else {
                        continue;
                    };
                    let moneda = campo(t, fila, "currency")
                        .and_then(Value::as_str)
                        .unwrap_or("Sin moneda");
                    let a = valores
                        .entry((gateway.to_owned(), moneda.trim().to_ascii_uppercase()))
                        .or_default();
                    for (i, nombre) in [
                        "volumen_capturado",
                        "capturados",
                        "autorizados_sin_capturar",
                    ]
                    .iter()
                    .enumerate()
                    {
                        a[periodo * 3 + i] += numero(campo(t, fila, nombre)).unwrap();
                    }
                }
            }
        }
        if s.renovaciones.completo {
            if let Some(t) = s
                .renovaciones
                .datos
                .as_ref()
                .filter(|t| valida(t, &["no_cobradas"]))
            {
                cobertura_renovaciones[periodo] += 1;
                for fila in &t.filas {
                    if let Some(gateway) = campo(t, fila, "gateway").and_then(Value::as_str) {
                        renovaciones.entry(gateway.to_owned()).or_default()[periodo] +=
                            numero(campo(t, fila, "no_cobradas")).unwrap();
                    }
                }
            }
        }
    }
    let comparable = cobertura == [dias; 2];
    let comparable_renovaciones = cobertura_renovaciones == [dias; 2];
    let mut gateways: BTreeMap<String, Value> = BTreeMap::new();
    for ((gateway, moneda), v) in valores {
        let g = gateways
            .entry(gateway.clone())
            .or_insert_with(|| json!({"gateway":gateway,"monedas":[]}));
        g["monedas"].as_array_mut().unwrap().push(json!({"moneda":moneda,"volumen":v[0].to_string(),"capturados":v[1].to_string(),"sin_capturar":v[2].to_string(),"volumen_anterior":v[3].to_string(),"cambio_volumen":comparable.then(||(v[0]-v[3]).to_string()),"cambio_capturados":comparable.then(||(v[1]-v[4]).to_string()),"cambio_sin_capturar":comparable.then(||(v[2]-v[5]).to_string())}));
    }
    for (gateway, v) in renovaciones {
        let g = gateways
            .entry(gateway.clone())
            .or_insert_with(|| json!({"gateway":gateway,"monedas":[]}));
        g["no_cobradas"] = json!(v[0].to_string());
        g["cambio_no_cobradas"] = json!(comparable_renovaciones.then(|| (v[0] - v[1]).to_string()));
    }
    for g in gateways.values_mut() {
        g["aprobacion"] = json!({"estado":"No disponible","causa":"Los registros guardados no cubren todos los intentos y rechazos del gateway."});
        g["reembolsos_fallidos"] = json!({"estado":"No disponible","causa":"No existe una serie completa y validada de reembolsos fallidos, incluido el flujo histórico."});
    }
    json!({"gateways":gateways.into_values().collect::<Vec<_>>(),"dias":dias,"cobertura":cobertura,"cobertura_renovaciones":cobertura_renovaciones,"comparable":comparable,"comparable_renovaciones":comparable_renovaciones})
}

pub fn autorizaciones(actual: &Fuente<Tabla>, anterior: Option<&Fuente<Tabla>>) -> Value {
    let tabla = |f: &Fuente<Tabla>| {
        f.completo
            && f.datos.as_ref().is_some_and(|t| {
                valida(t, &["autorizados_sin_capturar", "mayores_siete_dias"])
                    && t.filas
                        .iter()
                        .all(|fila| campo(t, fila, "currency").is_some_and(Value::is_string))
            })
    };
    let anterior = anterior.filter(|f| tabla(f));
    let mut filas = BTreeMap::<(String, String), [Decimal; 4]>::new();
    if tabla(actual) {
        for (periodo, fuente) in std::iter::once((0, actual)).chain(anterior.map(|f| (1, f))) {
            let t = fuente.datos.as_ref().unwrap();
            for fila in &t.filas {
                let clave = (
                    campo(t, fila, "gateway").unwrap().as_str().unwrap().into(),
                    campo(t, fila, "currency").unwrap().as_str().unwrap().into(),
                );
                let valores = filas.entry(clave).or_default();
                valores[periodo * 2] = numero(campo(t, fila, "autorizados_sin_capturar")).unwrap();
                valores[periodo * 2 + 1] = numero(campo(t, fila, "mayores_siete_dias")).unwrap();
            }
        }
    }
    json!({"actualizado":actual.actualizado,"actualizado_anterior":anterior.map(|f|f.actualizado),"error":if tabla(actual){None}else{Some(actual.error.as_deref().unwrap_or("No hay una medición completa y válida"))},"filas":filas.into_iter().map(|((gateway,moneda),v)|json!({"gateway":gateway,"moneda":moneda,"sin_capturar":v[0].to_string(),"mayores_siete_dias":v[1].to_string(),"cambio":anterior.map(|_|(v[0]-v[2]).to_string()),"cambio_mayores_siete_dias":anterior.map(|_|(v[1]-v[3]).to_string())})).collect::<Vec<_>>()})
}

pub fn rendimiento(
    f: &Fuente<Tabla>,
    cobertura: &Fuente<Tabla>,
    dias: i64,
    fin: NaiveDate,
) -> Value {
    let off = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
    let fechas = cobertura
        .datos
        .as_ref()
        .and_then(|t| t.filas.first().map(|fila| (t, fila)))
        .and_then(|(t, fila)| {
            let fecha = |campo_nombre| {
                campo(t, fila, campo_nombre)
                    .and_then(Value::as_i64)
                    .and_then(chrono::DateTime::from_timestamp_millis)
                    .map(|f| f.with_timezone(&off).date_naive())
            };
            Some((fecha("primer_evento")?, fecha("ultimo_evento")?))
        });
    let inicio = fin - Duration::days(dias - 1);
    let anterior = inicio - Duration::days(dias);
    let completo = f.completo
        && cobertura.completo
        && fechas.is_some_and(|(desde, hasta)| desde <= inicio && hasta >= fin);
    let completo_anterior = completo && fechas.is_some_and(|(desde, _)| desde <= anterior);
    let periodo = if dias == 7 {
        "semana"
    } else if dias == 15 {
        "quincena"
    } else {
        "mes"
    };
    let mut filas = vec![];
    if let Some(t) = &f.datos {
        if t.filas.iter().any(|fila| {
            !["actual", "anterior"].iter().all(|lado| {
                campo(t, fila, &format!("{periodo}_{lado}_transacciones"))
                    .and_then(Value::as_u64)
                    .is_some()
            })
        }) {
            return json!({"actualizado":f.actualizado,"error":"La respuesta de New Relic no contiene los conteos esperados; no se reemplazan por cero","error_cobertura":cobertura.error,"desde_observado":fechas.map(|(d,_)|d),"hasta_observado":fechas.map(|(_,h)|h),"completo":false,"completo_anterior":false,"filas":[]});
        }
        for fila in &t.filas {
            let valor = |lado: &str, c: &str| campo(t, fila, &format!("{periodo}_{lado}_{c}"));
            let cantidad = |lado: &str| {
                valor(lado, "transacciones")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
            };
            let p95 = |lado: &str| {
                (cantidad(lado) > 0)
                    .then(|| {
                        valor(lado, "p95_segundos")
                            .and_then(|v| v.get("95"))
                            .and_then(Value::as_f64)
                    })
                    .flatten()
            };
            let tasa = |lado: &str| {
                (cantidad(lado) > 0)
                    .then(|| valor(lado, "errores_porcentaje").and_then(Value::as_f64))
                    .flatten()
            };
            let facet = campo(t, fila, "facet").and_then(Value::as_array);
            let comparar = completo_anterior && cantidad("actual") > 0 && cantidad("anterior") > 0;
            filas.push(json!({"aplicacion":facet.and_then(|f|f.first()).cloned(),"transaccion":facet.and_then(|f|f.get(1)).cloned(),"transacciones":if cantidad("actual")>0||completo{Some(cantidad("actual"))}else{None},"p95_segundos":p95("actual"),"errores_porcentaje":tasa("actual"),"p95_anterior":if comparar{p95("anterior")}else{None},"errores_anterior":if comparar{tasa("anterior")}else{None},"cambio_p95":if comparar{p95("actual").zip(p95("anterior")).map(|(a,b)|a-b)}else{None},"cambio_errores":if comparar{tasa("actual").zip(tasa("anterior")).map(|(a,b)|a-b)}else{None}}));
        }
    }
    json!({"actualizado":f.actualizado,"error":f.error,"error_cobertura":cobertura.error,"desde_observado":fechas.map(|(d,_)|d),"hasta_observado":fechas.map(|(_,h)|h),"completo":completo,"completo_anterior":completo_anterior,"filas":filas})
}

pub fn errores(fuente: Option<&Fuente<Fallos>>, inicio: NaiveDate, fin: NaiveDate) -> Value {
    let Some(f) = fuente.and_then(|f| f.datos.as_ref()) else {
        return json!([]);
    };
    let anterior = inicio - Duration::days((fin - inicio).num_days() + 1);
    let mut familias: BTreeMap<String, Value> = BTreeMap::new();
    for f in &f.familias {
        let g = familias.entry(f.clase.clone()).or_insert_with(||json!({"clase":f.clase,"nuevas":0,"abiertas":0,"resueltas":0,"ignoradas":0,"avisos":0,"avisos_anteriores":0,"dias":{},"casos":[]}));
        let creado = f
            .creado
            .with_timezone(&chrono::FixedOffset::west_opt(4 * 3600).unwrap())
            .date_naive();
        for (nombre, condicion) in [
            ("nuevas", creado >= inicio && creado <= fin),
            ("abiertas", !f.resuelto && !f.ignorado),
            ("resueltas", f.resuelto),
            ("ignoradas", f.ignorado),
        ] {
            if condicion {
                g[nombre] = json!(g[nombre].as_u64().unwrap() + 1);
            }
        }
        for (dia, n) in &f.avisos_por_dia {
            let key = if *dia >= inicio && *dia <= fin {
                Some("avisos")
            } else if *dia >= anterior && *dia < inicio {
                Some("avisos_anteriores")
            } else {
                None
            };
            if let Some(key) = key {
                g[key] = json!(g[key].as_u64().unwrap() + n);
            }
            if *dia >= anterior && *dia <= fin {
                let fecha = dia.to_string();
                g["dias"][&fecha] = json!(g["dias"][&fecha].as_u64().unwrap_or(0) + n);
            }
        }
        g["casos"].as_array_mut().unwrap().push(json!({"url":f.url,"actualizado":f.actualizado,"resuelto":f.resuelto,"ignorado":f.ignorado,"cerrado":f.cerrado,"pr_cierre":f.pr_cierre}));
    }
    json!(familias.into_values().collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saldo_compara_mediciones_sin_sumar_y_no_inventa_ceros_por_fallos() {
        let fuente = |filas| {
            Fuente::ok(
                Tabla {
                    columnas: [
                        "gateway",
                        "currency",
                        "autorizados_sin_capturar",
                        "mayores_siete_dias",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                    filas,
                    consulta: String::new(),
                    notas: vec![],
                },
                vec![],
            )
        };
        let mut actual = fuente(vec![vec![
            json!("stripe"),
            json!("USD"),
            json!(5),
            json!(1),
        ]]);
        let mut anterior = fuente(vec![
            vec![json!("stripe"), json!("USD"), json!(8), json!(3)],
            vec![json!("stripe"), json!("CAD"), json!(2), json!(1)],
        ]);
        let r = autorizaciones(&actual, Some(&anterior));
        let xs = r["filas"].as_array().unwrap();
        let usd = xs.iter().find(|x| x["moneda"] == "USD").unwrap();
        assert_eq!(usd["sin_capturar"], "5");
        assert_eq!(usd["cambio"], "-3");
        let cad = xs.iter().find(|x| x["moneda"] == "CAD").unwrap();
        assert_eq!(cad["sin_capturar"], "0");
        assert_eq!(cad["cambio"], "-2");
        anterior.completo = false;
        let r = autorizaciones(&actual, Some(&anterior));
        assert_eq!(r["filas"].as_array().unwrap().len(), 1);
        assert!(r["filas"][0]["cambio"].is_null());
        actual.completo = false;
        let r = autorizaciones(&actual, None);
        assert!(r["filas"].as_array().unwrap().is_empty());
        assert!(r["error"].is_string());
    }
    #[test]
    fn percentiles_sin_eventos_o_sin_historia_no_se_comparan() {
        let fin = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let tabla = |columnas: Vec<&str>, fila: Vec<Value>| {
            Fuente::ok(
                Tabla {
                    columnas: columnas.into_iter().map(str::to_owned).collect(),
                    filas: vec![fila],
                    consulta: String::new(),
                    notas: vec![],
                },
                vec![],
            )
        };
        let f = tabla(
            vec![
                "facet",
                "semana_actual_transacciones",
                "semana_actual_p95_segundos",
                "semana_actual_errores_porcentaje",
                "semana_anterior_transacciones",
                "semana_anterior_p95_segundos",
                "semana_anterior_errores_porcentaje",
            ],
            vec![
                json!(["PaybyCourt", "CapturePaymentJob"]),
                json!(100),
                json!({"95":0.1}),
                json!(1),
                json!(0),
                json!({"95":0}),
                Value::Null,
            ],
        );
        let ts = |dia: u32| {
            NaiveDate::from_ymd_opt(2026, 11, dia)
                .unwrap()
                .and_hms_opt(12, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis()
        };
        let mut c = tabla(
            vec!["primer_evento", "ultimo_evento"],
            vec![json!(ts(11)), json!(ts(18))],
        );
        let r = rendimiento(&f, &c, 7, fin);
        assert_eq!(r["completo"], true);
        assert_eq!(r["completo_anterior"], false);
        assert!(r["filas"][0]["p95_anterior"].is_null());
        c.datos.as_mut().unwrap().filas[0][0] = json!(ts(1));
        let r = rendimiento(&f, &c, 7, fin);
        assert_eq!(r["completo_anterior"], true);
        assert!(r["filas"][0]["cambio_p95"].is_null());
        assert!(r["filas"][0]["p95_anterior"].is_null());
    }
    #[test]
    fn comparaciones_exigen_cobertura_y_separan_monedas() {
        let dia = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let snap = |fecha, usd: &str| Snapshot {
            fecha,
            actualizado: chrono::Utc::now(),
            salud: Fuente::ok(
                Tabla {
                    columnas: vec![
                        "gateway".into(),
                        "currency".into(),
                        "volumen_capturado".into(),
                        "capturados".into(),
                        "autorizados_sin_capturar".into(),
                    ],
                    filas: vec![
                        vec![
                            json!("stripe"),
                            json!("USD"),
                            json!(usd),
                            json!(2),
                            json!(1),
                        ],
                        vec![
                            json!("stripe"),
                            json!("BOB"),
                            json!("12.10"),
                            json!(3),
                            json!(0),
                        ],
                    ],
                    consulta: String::new(),
                    notas: vec![],
                },
                vec![],
            ),
            renovaciones: Fuente::fallo("Sin datos"),
            rendimiento: Fuente::fallo("Sin datos"),
            errores: Fuente::fallo("Sin datos"),
        };
        let h = vec![snap(dia, "0.10"), snap(dia - Duration::days(1), "0.20")];
        let v = salud(&h, dia, dia);
        assert_eq!(v["gateways"][0]["monedas"][0]["volumen"], "12.10");
        assert_eq!(v["gateways"][0]["monedas"][1]["cambio_volumen"], "-0.10");
        let parcial = salud(&h[..1], dia, dia);
        assert!(parcial["gateways"][0]["monedas"][1]["cambio_volumen"].is_null());
        assert_eq!(
            parcial["gateways"][0]["aprobacion"]["estado"],
            "No disponible"
        );
        let mut variantes = h.clone();
        variantes[1].salud.datos.as_mut().unwrap().filas[0][1] = json!("usd");
        let normalizado = salud(&variantes, dia, dia);
        assert_eq!(
            normalizado["gateways"][0]["monedas"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(normalizado["gateways"][0]["monedas"][1]["moneda"], "USD");
        assert_eq!(
            normalizado["gateways"][0]["monedas"][1]["cambio_volumen"],
            "-0.10"
        );
        let mut invalido = h;
        invalido[0].salud.datos.as_mut().unwrap().filas[0][2] = json!("no disponible");
        assert_eq!(salud(&invalido, dia, dia)["cobertura"][0], 0);
    }
}
