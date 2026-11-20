use crate::{
    archivos,
    config::Config,
    modelo::{Datos, Edicion},
    proceso,
};
use anyhow::Result;
use chrono::NaiveDate;

pub fn ejecutar(c: &Config, fecha: NaiveDate) -> Result<()> {
    let dir = archivos::dia(&c.salida, fecha);
    let e: Edicion = archivos::leer(&dir.join("edicion.json"))?;
    let d: Datos = archivos::leer(&dir.join("datos.json"))?;
    crate::editar::validar(&e, &d)?;
    let velocidad = e
        .guion
        .split_whitespace()
        .count()
        .div_ceil(2)
        .clamp(130, 170);
    let aiff = dir.join("voz.aiff");
    let m4a = dir.join("voz.m4a");
    let result = (|| {
        proceso::ejecutar(
            "say",
            &[
                "-v".into(),
                c.voz.clone(),
                "-r".into(),
                velocidad.to_string(),
                "-o".into(),
                aiff.to_string_lossy().into(),
            ],
            &e.guion,
            90,
        )?;
        proceso::ejecutar(
            "afconvert",
            &[
                "-f".into(),
                "m4af".into(),
                "-d".into(),
                "aac".into(),
                aiff.to_string_lossy().into(),
                m4a.to_string_lossy().into(),
            ],
            "",
            30,
        )?;
        archivos::atomico(
            &c.salida.join(format!("{fecha}.m4a")),
            &std::fs::read(&m4a)?,
        )?;
        let _ = std::fs::remove_file(dir.join("audio-pendiente.txt"));
        let _ = std::fs::remove_file(dir.join("audio-error.txt"));
        Ok(())
    })();
    let _ = std::fs::remove_file(&aiff);
    let _ = std::fs::remove_file(&m4a);
    result
}
