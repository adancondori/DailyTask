use chrono::{DateTime, NaiveDate, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fuente<T> {
    pub actualizado: DateTime<Utc>,
    pub datos: Option<T>,
    pub error: Option<String>,
    pub evidencia: Vec<String>,
    pub completo: bool,
}
impl<T> Fuente<T> {
    pub fn ok(datos: T, evidencia: Vec<String>) -> Self {
        Self {
            actualizado: Utc::now(),
            datos: Some(datos),
            error: None,
            evidencia,
            completo: true,
        }
    }
    pub fn fallo(error: impl std::fmt::Display) -> Self {
        Self {
            actualizado: Utc::now(),
            datos: None,
            error: Some(format!("{error:#}")),
            evidencia: vec![],
            completo: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum Estado {
    #[serde(rename = "Requiere acción")]
    Accion,
    #[serde(rename = "En curso")]
    EnCurso,
    Bloqueado,
    Listo,
    Informativo,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub titulo: String,
    pub detalle: String,
    pub url: String,
    pub persona: String,
    pub estado: Estado,
    pub fecha: DateTime<Utc>,
    pub sospechoso: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pr {
    pub numero: u64,
    pub repo: String,
    pub titulo: String,
    pub url: String,
    pub autor: String,
    pub creado: DateTime<Utc>,
    pub actualizado: DateTime<Utc>,
    pub ultimo_commit: Option<DateTime<Utc>>,
    pub mergeado: Option<DateTime<Utc>>,
    pub reviews: Vec<Review>,
    pub solicitado: Vec<String>,
    pub ci: String,
    pub conflictos: bool,
    pub borrador: bool,
    pub abierto: bool,
    pub tickets: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Review {
    pub autor: String,
    pub fecha: DateTime<Utc>,
    pub estado: String,
    #[serde(default)]
    pub bot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Github {
    pub prs: Vec<Pr>,
    pub actividad: Vec<Item>,
    pub completo: bool,
    pub advertencias: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub clave: String,
    pub titulo: String,
    pub url: String,
    pub persona: String,
    pub github: Option<String>,
    pub estado: String,
    pub categoria: String,
    pub actualizado: DateTime<Utc>,
    pub iniciado: Option<DateTime<Utc>>,
    pub cerrado: Option<DateTime<Utc>>,
    pub prs: Vec<String>,
    pub cambios: Vec<Item>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sprint {
    pub id: String,
    pub nombre: String,
    pub inicio: NaiveDate,
    pub fin: NaiveDate,
    pub comprometidos: Vec<String>,
    pub terminados: Vec<String>,
    pub arrastrados: Vec<String>,
    pub evidencia: String,
    #[serde(default)]
    pub completo: bool,
    #[serde(default)]
    pub estado: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Jira {
    pub tickets: Vec<Ticket>,
    pub sprints: Vec<Sprint>,
    pub personas: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evento {
    pub item: Item,
    pub inicio: DateTime<Utc>,
    pub fin: DateTime<Utc>,
    pub cancelado: bool,
    #[serde(default)]
    pub calendario_id: String,
    #[serde(default)]
    pub ocupa_agenda: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Calendario {
    pub eventos: Vec<Evento>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Correo {
    pub item: Item,
    pub clase: String,
    pub tipo_maquina: Option<String>,
    pub error_clase: Option<String>,
    pub pago: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Gmail {
    pub correos: Vec<Correo>,
    pub promociones: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mensaje {
    pub item: Item,
    pub canal: String,
    pub mencion: bool,
    pub dm: bool,
    pub espera_respuesta: bool,
    pub resuelto: bool,
    pub deploy_prs: Vec<String>,
    pub produccion: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Slack {
    pub mensajes: Vec<Mensaje>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Noticia {
    pub titulo: String,
    pub resumen: String,
    pub url: String,
    pub medio: String,
    pub fecha: NaiveDate,
    pub tema: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Noticias {
    pub notas: Vec<Noticia>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Clima {
    pub actual: f64,
    pub maxima: f64,
    pub minima: f64,
    pub lluvia: f64,
    pub url: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dolar {
    pub oficial: f64,
    pub oficial_fecha: NaiveDate,
    pub paralelo: f64,
    pub muestras: usize,
    pub brecha: f64,
    pub cambio_diario: Option<f64>,
    pub cambio_cinco_dias: Option<f64>,
    pub oficial_url: String,
    pub paralelo_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Alerta {
    pub titulo: String,
    pub url: String,
    pub persona: String,
    pub estado: Estado,
    pub dias: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Datos {
    pub version: u32,
    #[serde(default)]
    pub usuario: String,
    pub fecha: NaiveDate,
    pub ayer_habil: NaiveDate,
    pub demo: bool,
    pub github: Fuente<Github>,
    pub clima: Fuente<Clima>,
    pub dolar: Fuente<Dolar>,
    pub gmail: Fuente<Gmail>,
    pub calendario: Fuente<Calendario>,
    pub slack: Fuente<Slack>,
    pub jira: Fuente<Jira>,
    pub noticias: Fuente<Noticias>,
    pub alertas: Vec<Alerta>,
    pub hecho_ayer: Vec<Item>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Texto {
    pub titulo: String,
    pub texto: String,
    pub referencia: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Edicion {
    pub fecha: NaiveDate,
    pub top: Vec<Texto>,
    pub resumenes: BTreeMap<String, String>,
    pub guion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Progreso {
    pub corriendo: bool,
    pub etapa: String,
    pub fuentes: BTreeMap<String, String>,
    pub error: Option<String>,
    pub actualizado: DateTime<Utc>,
}
