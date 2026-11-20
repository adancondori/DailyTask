use anyhow::{bail, Context, Result};
use chrono::{Datelike, Duration, FixedOffset, NaiveDate, Utc, Weekday};
use fs2::FileExt;
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub fn hoy() -> NaiveDate {
    Utc::now()
        .with_timezone(&FixedOffset::west_opt(4 * 3600).unwrap())
        .date_naive()
}
pub fn anterior_habil(fecha: NaiveDate) -> NaiveDate {
    let mut d = fecha - Duration::days(1);
    while matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
        d -= Duration::days(1);
    }
    d
}
pub fn dia(base: &Path, fecha: NaiveDate) -> PathBuf {
    base.join(fecha.to_string())
}
pub fn atomico(path: &Path, contenido: &[u8]) -> Result<()> {
    let padre = path.parent().context("Archivo sin directorio")?;
    fs::create_dir_all(padre)?;
    let tmp = padre.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(contenido)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
pub fn guardar<T: Serialize>(path: &Path, dato: &T) -> Result<()> {
    atomico(path, &serde_json::to_vec_pretty(dato)?)
}
pub fn leer<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_slice(&fs::read(path).with_context(|| format!("Falta {}", path.display()))?)
        .context("JSON inválido")
}
pub struct Candado(File);
impl Candado {
    pub fn tomar(base: &Path) -> Result<Self> {
        fs::create_dir_all(base)?;
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(base.join(".corrida.lock"))?;
        if f.try_lock_exclusive().is_err() {
            bail!("Ya hay una corrida en curso");
        }
        Ok(Self(f))
    }
}
impl Drop for Candado {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lunes_y_anio() {
        assert_eq!(
            anterior_habil(NaiveDate::from_ymd_opt(2027, 1, 4).unwrap()).to_string(),
            "2027-01-01"
        );
        assert_eq!(
            anterior_habil(NaiveDate::from_ymd_opt(2026, 11, 23).unwrap()).to_string(),
            "2026-11-20"
        );
    }
    #[test]
    fn exclusion() {
        let t = tempfile::tempdir().unwrap();
        let a = Candado::tomar(t.path()).unwrap();
        assert!(Candado::tomar(t.path()).is_err());
        drop(a);
        assert!(Candado::tomar(t.path()).is_ok());
    }
}
