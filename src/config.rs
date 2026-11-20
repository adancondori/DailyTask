use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub ciudad: String,
    pub latitud: f64,
    pub longitud: f64,
    pub zona_horaria: String,
    pub salida: PathBuf,
    pub github_usuario: String,
    pub github_organizacion: String,
    pub repositorios: Vec<String>,
    pub jira_sitio: String,
    pub jira_proyecto: String,
    pub jira_tablero: u64,
    #[serde(default)]
    pub jira_filtro: Option<u64>,
    #[serde(default)]
    pub jira_personas: Vec<String>,
    pub personas_primero: Vec<String>,
    pub canales_bot: Vec<String>,
    pub canal_deploy: String,
    pub temas: Vec<String>,
    pub puerto: u16,
    pub claude_modelo: String,
    pub timeout_segundos: u64,
    pub voz: String,
    pub reporte_url: String,
    pub mcp_config: String,
    pub identidades: BTreeMap<String, String>,
}
impl Config {
    pub fn cargar(path: &Path, demo: bool) -> Result<Self> {
        let mut c: Self =
            toml::from_str(&std::fs::read_to_string(path).context("No se pudo leer config.toml")?)?;
        let base = path.canonicalize()?.parent().unwrap().to_owned();
        if c.salida.is_relative() {
            c.salida = base.join(&c.salida);
        }
        if demo {
            c.salida = c.salida.join("demo");
        }
        if c.zona_horaria != "America/La_Paz" {
            bail!("Esta versión requiere America/La_Paz (UTC-4)");
        }
        if c.puerto == 0 || c.timeout_segundos == 0 {
            bail!("Puerto y timeout deben ser positivos");
        }
        if !c.reporte_url.is_empty() && !c.reporte_url.starts_with("https://claude.ai/") {
            bail!("reporte_url debe ser una URL HTTPS de claude.ai");
        }
        Ok(c)
    }
}
