mod alertas;
mod archivos;
mod audio;
mod config;
mod editar;
mod equipo;
mod modelo;
mod proceso;
mod recolectar;
mod render;
mod servir;

use anyhow::Result;
use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use config::Config;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(version, about = "El Diario: periódico personal local")]
struct Cli {
    #[arg(long, global = true, default_value = "config.toml")]
    config: PathBuf,
    #[arg(long, global = true)]
    demo: bool,
    #[arg(long, global = true)]
    fecha: Option<NaiveDate>,
    #[arg(long, global = true)]
    sin_audio: bool,
    #[command(subcommand)]
    comando: Option<Comando>,
}
#[derive(Subcommand, Debug)]
enum Comando {
    Recolectar {
        #[arg(long)]
        fuente: Option<String>,
    },
    Editar,
    Render,
    Audio,
    Equipo {
        #[arg(long)]
        publicar: bool,
        #[arg(long, conflicts_with = "publicar")]
        solo_edicion: bool,
    },
    Servir,
    Todo,
    Verificar,
    Instalar,
    #[command(hide = true)]
    Permiso {
        #[arg(long, allow_hyphen_values = true)]
        permitidas: String,
        #[arg(long)]
        consultas_json: Option<String>,
        #[arg(long)]
        lectura: Option<PathBuf>,
        #[arg(long)]
        entrada_json: Option<String>,
    },
}
fn main() {
    if let Err(e) = run() {
        eprintln!("El Diario: {e:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Some(Comando::Permiso {
        permitidas,
        consultas_json,
        lectura,
        entrada_json,
    }) = &cli.comando
    {
        if let Err(e) = recolectar::claude::permiso(
            permitidas,
            consultas_json.as_deref(),
            lectura.as_deref(),
            entrada_json.as_deref(),
        ) {
            eprintln!("El Diario bloqueó la herramienta: {e:#}");
            std::process::exit(2);
        }
        return Ok(());
    }
    let c = Config::cargar(&cli.config, cli.demo)?;
    let fecha = cli.fecha.unwrap_or_else(archivos::hoy);
    let comando = cli.comando.unwrap_or(Comando::Todo);
    match comando {
        Comando::Servir => return servir::ejecutar(c, cli.config, cli.demo, cli.sin_audio),
        Comando::Verificar => return equipo::verificar_conexiones(&c),
        Comando::Instalar => return servir::instalar(&c, &cli.config),
        _ => {}
    }
    let _lock = archivos::Candado::tomar(&c.salida)?;
    recolectar::progreso(&c.salida, "inicio", None, true, None)?;
    let resultado = (|| {
        match comando {
            Comando::Recolectar { fuente } => {
                if let Some(fuente) = fuente {
                    recolectar::actualizar(&c, fecha, cli.demo, &fuente)?;
                } else {
                    recolectar::ejecutar(&c, fecha, cli.demo)?;
                }
            }
            Comando::Editar => {
                editar::ejecutar(&c, fecha, cli.demo)?;
            }
            Comando::Render => {
                render::ejecutar(&c, fecha)?;
            }
            Comando::Audio => {
                audio::ejecutar(&c, fecha)?;
                render::ejecutar(&c, fecha)?;
            }
            Comando::Equipo {
                publicar,
                solo_edicion,
            } => {
                if solo_edicion {
                    return equipo::reeditar(&c, fecha, cli.demo);
                }
                if publicar && !cli.demo {
                    for fuente in ["github", "jira", "slack"] {
                        recolectar::actualizar(&c, fecha, false, fuente)?;
                    }
                }
                equipo::ejecutar(&c, fecha, cli.demo, publicar)?;
            }
            Comando::Todo => {
                recolectar::ejecutar(&c, fecha, cli.demo)?;
                recolectar::progreso(&c.salida, "equipo", None, true, None)?;
                if let Err(e) = equipo::ejecutar(&c, fecha, cli.demo, false) {
                    eprintln!("Reporte: {e:#}");
                }
                recolectar::progreso(&c.salida, "editar", None, true, None)?;
                editar::ejecutar(&c, fecha, cli.demo)?;
                render::ejecutar(&c, fecha)?;
                if !cli.sin_audio {
                    recolectar::progreso(&c.salida, "audio", None, true, None)?;
                    if let Err(e) = audio::ejecutar(&c, fecha) {
                        archivos::atomico(
                            &archivos::dia(&c.salida, fecha).join("audio-error.txt"),
                            e.to_string().as_bytes(),
                        )?;
                        eprintln!("Audio no disponible: {e}");
                    }
                    render::ejecutar(&c, fecha)?;
                }
            }
            _ => unreachable!(),
        };
        Ok(())
    })();
    let error = resultado
        .as_ref()
        .err()
        .map(|e: &anyhow::Error| format!("{e:#}"));
    recolectar::progreso(
        &c.salida,
        if error.is_none() { "Listo" } else { "Falló" },
        None,
        false,
        error,
    )?;
    resultado
}
