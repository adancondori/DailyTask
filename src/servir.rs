use crate::{archivos, config::Config, modelo::Progreso, recolectar};
use anyhow::{bail, Context, Result};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

fn cabecera(nombre: &str, valor: &str) -> Header {
    Header::from_bytes(nombre.as_bytes(), valor.as_bytes()).unwrap()
}
fn header<'a>(r: &'a Request, nombre: &'static str) -> Option<&'a str> {
    r.headers()
        .iter()
        .find(|h| h.field.equiv(nombre))
        .map(|h| h.value.as_str())
}
fn json(r: Request, status: u16, v: serde_json::Value) {
    let _ = r.respond(
        Response::from_string(v.to_string())
            .with_status_code(StatusCode(status))
            .with_header(cabecera("Content-Type", "application/json; charset=utf-8"))
            .with_header(cabecera("Cache-Control", "no-store"))
            .with_header(cabecera("X-Content-Type-Options", "nosniff")),
    );
}
pub fn autorizada(
    host: Option<&str>,
    origin: Option<&str>,
    marca: Option<&str>,
    puerto: u16,
) -> bool {
    host == Some(format!("127.0.0.1:{puerto}").as_str())
        && origin == Some(format!("http://127.0.0.1:{puerto}").as_str())
        && marca == Some("1")
}
pub fn ruta(base: &Path, url: &str) -> Result<PathBuf> {
    let raw = url.split('?').next().unwrap_or("/");
    let decoded = percent_encoding::percent_decode_str(raw).decode_utf8()?;
    let name = if decoded == "/" {
        "ultimo.html"
    } else {
        decoded.trim_start_matches('/')
    };
    let path = Path::new(name);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
    {
        bail!("Ruta rechazada");
    }
    let root = base.canonicalize()?;
    let dest = root.join(path).canonicalize()?;
    if !dest.starts_with(&root) || !dest.is_file() {
        bail!("Ruta rechazada");
    }
    Ok(dest)
}
pub fn ejecutar(c: Config, config_path: PathBuf, demo: bool, sin_audio: bool) -> Result<()> {
    fs::create_dir_all(&c.salida)?;
    let config_path = config_path.canonicalize()?;
    let addr = format!("127.0.0.1:{}", c.puerto);
    let server =
        Server::http(&addr).map_err(|e| anyhow::anyhow!("No se pudo escuchar en {addr}: {e}"))?;
    let busy = Arc::new(AtomicBool::new(false));
    eprintln!("El Diario: http://{addr}/ultimo.html");
    for request in server.incoming_requests() {
        let host = format!("127.0.0.1:{}", c.puerto);
        if header(&request, "Host") != Some(host.as_str()) {
            json(request, 403, serde_json::json!({"error":"Host rechazado"}));
            continue;
        }
        let endpoint = request.url().split('?').next().unwrap_or("").to_owned();
        if endpoint == "/api/estado" && request.method() == &Method::Get {
            let mut p: Progreso =
                archivos::leer(&c.salida.join("progreso.json")).unwrap_or(Progreso {
                    corriendo: false,
                    etapa: "Listo".into(),
                    fuentes: Default::default(),
                    error: None,
                    actualizado: chrono::Utc::now(),
                });
            let locked = archivos::Candado::tomar(&c.salida).is_err();
            p.corriendo = busy.load(Ordering::SeqCst) || locked;
            json(request, 200, serde_json::to_value(&p)?);
            continue;
        }
        if endpoint == "/api/sync" || endpoint == "/api/reporte" {
            if request.method() != &Method::Post {
                json(request, 405, serde_json::json!({"error":"Usa POST"}));
                continue;
            }
            if !autorizada(
                header(&request, "Host"),
                header(&request, "Origin"),
                header(&request, "X-Diario"),
                c.puerto,
            ) {
                json(
                    request,
                    403,
                    serde_json::json!({"error":"Acción permitida solo desde la página local"}),
                );
                continue;
            }
            if request.body_length().unwrap_or(usize::MAX) > 4096 {
                json(
                    request,
                    413,
                    serde_json::json!({"error":"Petición demasiado grande"}),
                );
                continue;
            }
            if busy.swap(true, Ordering::SeqCst) {
                json(
                    request,
                    409,
                    serde_json::json!({"error":"Ya hay una corrida en curso"}),
                );
                continue;
            }
            match archivos::Candado::tomar(&c.salida) {
                Ok(lock) => drop(lock),
                Err(_) => {
                    busy.store(false, Ordering::SeqCst);
                    json(
                        request,
                        409,
                        serde_json::json!({"error":"Ya hay una corrida CLI o programada"}),
                    );
                    continue;
                }
            }
            let is_report = endpoint == "/api/reporte";
            if demo && is_report {
                busy.store(false, Ordering::SeqCst);
                json(
                    request,
                    400,
                    serde_json::json!({"error":"El demo no publica información"}),
                );
                continue;
            }
            recolectar::progreso(&c.salida, "Iniciando", None, true, None)?;
            let cc = c.clone();
            let cp = config_path.clone();
            let running = busy.clone();
            thread::spawn(move || {
                let result = (|| -> Result<()> {
                    let mut cmd = Command::new(std::env::current_exe()?);
                    cmd.arg("--config").arg(cp);
                    if demo {
                        cmd.arg("--demo");
                    }
                    if sin_audio {
                        cmd.arg("--sin-audio");
                    }
                    if is_report {
                        cmd.args(["equipo", "--publicar"]);
                    } else {
                        cmd.arg("todo");
                    }
                    let s = cmd.status()?;
                    if !s.success() {
                        bail!("La corrida terminó con errores; revisa el detalle y las fuentes");
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    let _ =
                        recolectar::progreso(&cc.salida, "Falló", None, false, Some(e.to_string()));
                }
                running.store(false, Ordering::SeqCst);
            });
            json(request, 202, serde_json::json!({"ok":true}));
            continue;
        }
        if request.method() != &Method::Get && request.method() != &Method::Head {
            json(
                request,
                405,
                serde_json::json!({"error":"Método no permitido"}),
            );
            continue;
        }
        match ruta(&c.salida, request.url()) {
            Ok(path) => {
                if let Err(e) = estatico(request, &path) {
                    eprintln!("HTTP: {e}");
                }
            }
            Err(_) => json(
                request,
                404,
                serde_json::json!({"error":"Archivo no disponible; genera una edición con diario todo"}),
            ),
        }
    }
    Ok(())
}
fn estatico(r: Request, path: &Path) -> Result<()> {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    let mime = match ext {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "m4a" => "audio/mp4",
        "md" | "txt" => "text/plain; charset=utf-8",
        _ => {
            json(r, 404, serde_json::json!({"error":"Tipo no disponible"}));
            return Ok(());
        }
    };
    let mut file = File::open(path)?;
    let total = file.metadata()?.len();
    let mut status = 200;
    let mut start = 0;
    let mut end = total.saturating_sub(1);
    if let Some(range) = header(&r, "Range") {
        let parts = range.strip_prefix("bytes=").and_then(|s| s.split_once('-'));
        if let Some((a, b)) = parts {
            if let Ok(s) = a.parse::<u64>() {
                start = s;
                if !b.is_empty() {
                    end = b.parse().unwrap_or(end);
                }
                status = 206;
            } else {
                json(r, 416, serde_json::json!({"error":"Rango inválido"}));
                return Ok(());
            }
        } else {
            json(r, 416, serde_json::json!({"error":"Rango inválido"}));
            return Ok(());
        }
    }
    if total == 0 || start > end || end >= total {
        json(
            r,
            416,
            serde_json::json!({"error":"Rango fuera del archivo"}),
        );
        return Ok(());
    }
    file.seek(SeekFrom::Start(start))?;
    let len = end - start + 1;
    let mut headers=vec![cabecera("Content-Type",mime),cabecera("Cache-Control","no-store"),cabecera("Accept-Ranges","bytes"),cabecera("X-Content-Type-Options","nosniff"),cabecera("Referrer-Policy","no-referrer"),cabecera("Content-Security-Policy","default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'")];
    if status == 206 {
        headers.push(cabecera(
            "Content-Range",
            &format!("bytes {start}-{end}/{total}"),
        ));
    }
    let response = Response::new(
        StatusCode(status),
        headers,
        file.take(len),
        Some(len as usize),
        None,
    );
    r.respond(response)?;
    Ok(())
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
pub fn instalar(c: &Config, path: &Path) -> Result<()> {
    if std::env::consts::OS != "macos" {
        bail!("launchd requiere macOS");
    }
    let config_path = path.canonicalize()?;
    let root = config_path.parent().context("Config sin directorio")?;
    let tz = crate::proceso::ejecutar("readlink", &["/etc/localtime".into()], "", 5)?;
    if !tz.trim().ends_with("America/La_Paz") {
        bail!("La zona del sistema no es America/La_Paz; launchd usa la zona del sistema. Ajusta la zona antes de instalar el horario de las 7:00 UTC-4");
    }
    let bin = std::env::current_exe()?.canonicalize()?;
    let log = c.salida.join("logs");
    fs::create_dir_all(&log)?;
    let agent_dir = PathBuf::from(std::env::var("HOME")?).join("Library/LaunchAgents");
    fs::create_dir_all(&agent_dir)?;
    for (label, template) in [
        (
            "com.diario.servidor",
            include_str!("../launchd/com.diario.plist"),
        ),
        (
            "com.diario.manana",
            include_str!("../launchd/com.diario.manana.plist"),
        ),
    ] {
        let s = template
            .replace("{{BINARIO}}", &xml(&bin.to_string_lossy()))
            .replace("{{CONFIG}}", &xml(&config_path.to_string_lossy()))
            .replace("{{RAIZ}}", &xml(&root.to_string_lossy()))
            .replace("{{LOGS}}", &xml(&log.to_string_lossy()));
        let dest = agent_dir.join(format!("{label}.plist"));
        archivos::atomico(&dest, s.as_bytes())?;
        let uid = unsafe { libc::getuid() };
        let dominio = format!("gui/{uid}");
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("{dominio}/{label}")])
            .output();
        for intento in 0..10 {
            let resultado = Command::new("launchctl")
                .arg("bootstrap")
                .arg(&dominio)
                .arg(&dest)
                .output()?;
            if resultado.status.success() {
                break;
            }
            if resultado.status.code() != Some(5) || intento == 9 {
                bail!(
                    "No se pudo cargar {label}; plist generado en {}: {}",
                    dest.display(),
                    String::from_utf8_lossy(&resultado.stderr).trim()
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        println!("Cargado {label}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csrf_y_host() {
        assert!(autorizada(
            Some("127.0.0.1:7331"),
            Some("http://127.0.0.1:7331"),
            Some("1"),
            7331
        ));
        assert!(!autorizada(
            Some("evil.test:7331"),
            Some("http://127.0.0.1:7331"),
            Some("1"),
            7331
        ));
        assert!(!autorizada(
            Some("127.0.0.1:7331"),
            Some("https://evil.test"),
            Some("1"),
            7331
        ));
        assert!(!autorizada(Some("127.0.0.1:7331"), None, Some("1"), 7331));
    }
    #[test]
    fn traversal() {
        let t = tempfile::tempdir().unwrap();
        fs::write(t.path().join("ultimo.html"), "ok").unwrap();
        assert!(ruta(t.path(), "/").is_ok());
        assert!(ruta(t.path(), "/%2e%2e/Cargo.toml").is_err());
        assert!(ruta(t.path(), "/.corrida.lock").is_err());
        let other = tempfile::NamedTempFile::new().unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(other.path(), t.path().join("fuga.json")).unwrap();
            assert!(ruta(t.path(), "/fuga.json").is_err());
        }
    }
}
