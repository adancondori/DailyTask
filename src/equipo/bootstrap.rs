use super::{Snapshot, Tabla};
use crate::{archivos, config::Config, modelo::Fuente, recolectar};
use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate, Utc};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
enum Tipo {
    Salud,
    Renovaciones,
}

impl Tipo {
    fn nombre(self) -> &'static str {
        match self {
            Self::Salud => "salud",
            Self::Renovaciones => "renovaciones",
        }
    }
    fn sql(self) -> &'static str {
        match self {
            Self::Salud => include_str!("../../consultas/salud_historia.sql"),
            Self::Renovaciones => include_str!("../../consultas/renovaciones_historia.sql"),
        }
    }
    fn fuente(self, s: &Snapshot) -> &Fuente<Tabla> {
        match self {
            Self::Salud => &s.salud,
            Self::Renovaciones => &s.renovaciones,
        }
    }
    fn poner(self, s: &mut Snapshot, f: Fuente<Tabla>) {
        match self {
            Self::Salud => s.salud = f,
            Self::Renovaciones => s.renovaciones = f,
        }
    }
}

fn directorio(base: &Path, dia: NaiveDate) -> PathBuf {
    base.join((dia + Duration::days(1)).to_string())
}

fn vacio(fecha: NaiveDate) -> Snapshot {
    Snapshot {
        fecha,
        actualizado: Utc::now(),
        salud: Fuente::fallo("No se recopiló salud para esta fecha"),
        renovaciones: Fuente::fallo("No se recopilaron renovaciones para esta fecha"),
        rendimiento: Fuente::fallo("No hay snapshot de rendimiento para esta fecha"),
        errores: Fuente::fallo("Las familias de errores se consultan en la última serie completa"),
    }
}

fn incorporar(
    base: &Path,
    tipo: Tipo,
    desde: NaiveDate,
    hasta: NaiveDate,
    f: &Fuente<Tabla>,
) -> Result<()> {
    anyhow::ensure!(f.completo, "La consulta histórica no está completa");
    let t = f.datos.as_ref().context("La historia no contiene datos")?;
    let columna = t
        .columnas
        .iter()
        .position(|c| c == "fecha")
        .context("La historia no expone fecha")?;
    let mut filas = std::collections::BTreeMap::<NaiveDate, Vec<Vec<serde_json::Value>>>::new();
    for fila in &t.filas {
        anyhow::ensure!(
            fila.len() == t.columnas.len(),
            "Columnas históricas inconsistentes"
        );
        let dia = NaiveDate::parse_from_str(
            fila[columna]
                .as_str()
                .context("Fecha histórica sin formato ISO")?,
            "%Y-%m-%d",
        )?;
        anyhow::ensure!(
            dia >= desde && dia < hasta,
            "Una fila histórica salió de la ventana autorizada"
        );
        let mut valores = fila.clone();
        valores.remove(columna);
        filas.entry(dia).or_default().push(valores);
    }
    let mut dia = desde;
    while dia < hasta {
        let dir = directorio(base, dia);
        let path = dir.join("snapshot.json");
        let mut s: Snapshot = if path.is_file() {
            archivos::leer(&path)?
        } else {
            vacio(dia)
        };
        anyhow::ensure!(s.fecha == dia, "Snapshot histórico fuera de fecha");
        if !tipo.fuente(&s).completo || tipo.fuente(&s).datos.is_none() {
            let mut diario = f.clone();
            let mut tabla = t.clone();
            tabla.columnas.remove(columna);
            tabla.filas = filas.remove(&dia).unwrap_or_default();
            tabla.notas.push(format!(
                "Cohorte creada el {dia}; estado observado al recopilar la consulta histórica."
            ));
            diario.datos = Some(tabla);
            diario.evidencia.push(format!(
                "Filas originales de la cohorte {dia}; sin recalcular valores"
            ));
            archivos::guardar(&dir.join(format!("{}.json", tipo.nombre())), &diario)?;
            tipo.poner(&mut s, diario);
            s.actualizado = Utc::now();
            archivos::guardar(&path, &s)?;
        }
        dia += Duration::days(1);
    }
    Ok(())
}

fn completar_tipo(c: &Config, fecha: NaiveDate, tipo: Tipo) -> Result<()> {
    let base = c.salida.join("equipo");
    let desde = fecha - Duration::days(60);
    let hasta = fecha - Duration::days(1);
    let mut faltantes = vec![];
    let mut dia = desde;
    while dia < hasta {
        let path = directorio(&base, dia).join("snapshot.json");
        let falta = if path.is_file() {
            let s: Snapshot = archivos::leer(&path)?;
            anyhow::ensure!(s.fecha == dia, "Snapshot histórico fuera de fecha");
            !tipo.fuente(&s).completo || tipo.fuente(&s).datos.is_none()
        } else {
            true
        };
        if falta {
            faltantes.push(dia);
        }
        dia += Duration::days(1);
    }
    let mut i = 0;
    while i < faltantes.len() {
        let inicio = faltantes[i];
        let mut fin = inicio + Duration::days(1);
        i += 1;
        while i < faltantes.len() && faltantes[i] == fin {
            fin += Duration::days(1);
            i += 1;
        }
        let q = tipo
            .sql()
            .replace("{{inicio}}", &format!("{inicio} 04:00:00"))
            .replace("{{fin}}", &format!("{fin} 04:00:00"))
            .trim()
            .to_owned();
        let nombre = format!("Historia de {}", tipo.nombre());
        let path = base
            .join("historia")
            .join(format!("{}-{inicio}-{fin}.json", tipo.nombre()));
        let f = super::tabla_guardada(c, &nombre, &q, crate::recolectar::claude::CLICKHOUSE, &path);
        incorporar(&base, tipo, inicio, fin, &f)?;
    }
    Ok(())
}

pub fn completar(c: &Config, fecha: NaiveDate) -> Vec<String> {
    let mut errores = vec![];
    for tipo in [Tipo::Salud, Tipo::Renovaciones] {
        if let Err(e) = completar_tipo(c, fecha, tipo) {
            let mensaje = format!("Historia de {} incompleta: {e:#}", tipo.nombre());
            let _ = recolectar::progreso(
                &c.salida,
                "equipo",
                Some((&format!("Historia de {}", tipo.nombre()), &mensaje)),
                true,
                None,
            );
            errores.push(mensaje);
        }
    }
    errores
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn conserva_archivos_completos_y_particiona_sin_perder_decimales() {
        let t = tempfile::tempdir().unwrap();
        let dia = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        let tabla = |filas| {
            Fuente::ok(
                Tabla {
                    columnas: vec![
                        "fecha".into(),
                        "gateway".into(),
                        "currency".into(),
                        "volumen_capturado".into(),
                    ],
                    filas,
                    consulta: "fixture".into(),
                    notas: vec![],
                },
                vec!["Fixture".into()],
            )
        };
        let f = tabla(vec![
            vec![
                json!(dia.to_string()),
                json!("demo"),
                json!("USD"),
                json!("10.00"),
            ],
            vec![
                json!((dia + Duration::days(1)).to_string()),
                json!("demo"),
                json!("USD"),
                json!("0.10"),
            ],
        ]);
        incorporar(t.path(), Tipo::Salud, dia, dia + Duration::days(2), &f).unwrap();
        let path = directorio(t.path(), dia).join("snapshot.json");
        let anterior = std::fs::read(&path).unwrap();
        let mut cambiada = f.clone();
        cambiada.datos.as_mut().unwrap().filas[0][3] = json!("999.99");
        incorporar(
            t.path(),
            Tipo::Salud,
            dia,
            dia + Duration::days(2),
            &cambiada,
        )
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), anterior);
        let s: Snapshot =
            archivos::leer(&directorio(t.path(), dia + Duration::days(1)).join("snapshot.json"))
                .unwrap();
        assert_eq!(s.salud.datos.unwrap().filas[0][2], "0.10");
        cambiada.datos.as_mut().unwrap().filas[0][0] = json!("1999-01-01");
        assert!(incorporar(
            t.path(),
            Tipo::Salud,
            dia,
            dia + Duration::days(2),
            &cambiada
        )
        .is_err());
    }
}
