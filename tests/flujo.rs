use fs2::FileExt;
use serde_json::Value;
use std::{
    fs,
    net::TcpListener,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn run(config: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_diario"))
        .args([
            "--config",
            config.to_str().unwrap(),
            "--demo",
            "--sin-audio",
        ])
        .args(args)
        .output()
        .unwrap()
}
fn preparar() -> (tempfile::TempDir, std::path::PathBuf, u16) {
    let t = tempfile::tempdir().unwrap();
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = t.path().join("config.toml");
    let content = include_str!("../config.example.toml")
        .replace(
            "salida = \"salida\"",
            &format!("salida = {:?}", t.path().join("salida").to_str().unwrap()),
        )
        .replace("puerto = 7331", &format!("puerto = {port}"));
    fs::write(&config, content).unwrap();
    (t, config, port)
}
#[test]
fn demo_etapas_y_preservacion() {
    let (t, c, _) = preparar();
    let result = run(&c, &["todo"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let base = t.path().join("salida/demo");
    let html = fs::read(base.join("ultimo.html")).unwrap();
    assert!(String::from_utf8_lossy(&html).contains("Demostración"));
    assert!(base.join("equipo/ultimo.html").is_file());
    let date = fs::read_dir(&base)
        .unwrap()
        .filter_map(Result::ok)
        .find(|p| p.path().join("edicion.json").is_file())
        .unwrap()
        .path();
    let edicion = date.join("edicion.json");
    let mut e: Value = serde_json::from_slice(&fs::read(&edicion).unwrap()).unwrap();
    e["top"][0]["titulo"] = "Titular editorial de comprobación".into();
    fs::write(&edicion, serde_json::to_vec(&e).unwrap()).unwrap();
    assert!(run(&c, &["render"]).status.success());
    let html = fs::read(base.join("ultimo.html")).unwrap();
    assert!(String::from_utf8_lossy(&html).contains("Titular editorial de comprobación"));
    e["top"][0]["referencia"] = "referencia-inventada".into();
    fs::write(&edicion, serde_json::to_vec(&e).unwrap()).unwrap();
    let failed = run(&c, &["render"]);
    assert!(!failed.status.success());
    assert_eq!(fs::read(base.join("ultimo.html")).unwrap(), html);
    let result = run(&c, &["editar"]);
    assert!(result.status.success());
    let datos = date.join("datos.json");
    let mut d: Value = serde_json::from_slice(&fs::read(&datos).unwrap()).unwrap();
    d["clima"]["datos"] = Value::Null;
    d["clima"]["error"] = "Fuente caída de prueba".into();
    d["gmail"]["datos"]["correos"][3]["item"]["titulo"] = "<script>alert('x')</script>".into();
    d["gmail"]["datos"]["correos"][3]["item"]["url"] = "javascript:alert(1)".into();
    fs::write(datos, serde_json::to_vec(&d).unwrap()).unwrap();
    assert!(run(&c, &["render"]).status.success());
    let new = fs::read_to_string(base.join("ultimo.html")).unwrap();
    assert!(new.contains("Fuente caída de prueba"));
    assert!(new.contains("&lt;script&gt;"));
    assert!(!new.contains("javascript:alert(1)"));
}

#[test]
fn redaccion_nueva_conserva_audio_anterior_hasta_reemplazo() {
    let (t, c, _) = preparar();
    assert!(run(&c, &["todo"]).status.success());
    let base = t.path().join("salida/demo");
    let dir = fs::read_dir(&base)
        .unwrap()
        .filter_map(Result::ok)
        .find(|p| p.path().join("edicion.json").is_file())
        .unwrap()
        .path();
    let path = dir.join("edicion.json");
    let mut e: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let audio = base.join(format!("{}.m4a", e["fecha"].as_str().unwrap()));
    e["guion"] = format!("{} actualizado", e["guion"].as_str().unwrap()).into();
    fs::write(&path, serde_json::to_vec(&e).unwrap()).unwrap();
    fs::write(&audio, b"audio anterior").unwrap();
    assert!(run(&c, &["editar"]).status.success());
    assert_eq!(fs::read(&audio).unwrap(), b"audio anterior");
    assert!(dir.join("audio-pendiente.txt").is_file());
    assert!(run(&c, &["render"]).status.success());
    assert!(!fs::read_to_string(base.join("ultimo.html"))
        .unwrap()
        .contains("<audio "));
}
#[test]
fn agenda_conserva_eventos_iniciados_antes_de_hoy_que_siguen_vigentes() {
    let (t, c, _) = preparar();
    assert!(run(&c, &["todo"]).status.success());
    let base = t.path().join("salida/demo");
    let dir = fs::read_dir(&base)
        .unwrap()
        .filter_map(Result::ok)
        .find(|p| p.path().join("datos.json").is_file())
        .unwrap()
        .path();
    let path = dir.join("datos.json");
    let mut d: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let fecha: chrono::NaiveDate = d["fecha"].as_str().unwrap().parse().unwrap();
    let instante = |dias| {
        (fecha + chrono::Duration::days(dias))
            .and_hms_opt(4, 0, 0)
            .unwrap()
            .and_utc()
            .to_rfc3339()
    };
    let mut evento = d["calendario"]["datos"]["eventos"][0].clone();
    evento["item"]["id"] = "ausencia-iniciada".into();
    evento["item"]["titulo"] = "Ausencia ya iniciada".into();
    evento["ocupa_agenda"] = false.into();
    evento["inicio"] = instante(-2).into();
    evento["fin"] = instante(2).into();
    let mut terminado = evento.clone();
    terminado["item"]["id"] = "evento-terminado".into();
    terminado["item"]["titulo"] = "Evento terminado antes de hoy".into();
    terminado["fin"] = instante(0).into();
    let eventos = d["calendario"]["datos"]["eventos"].as_array_mut().unwrap();
    let mut propia = evento.clone();
    propia["item"]["id"] = "reunion-propia".into();
    propia["item"]["titulo"] = "Reunión propia sin choque".into();
    propia["inicio"] = fecha
        .and_hms_opt(22, 0, 0)
        .unwrap()
        .and_utc()
        .to_rfc3339()
        .into();
    propia["fin"] = fecha
        .and_hms_opt(23, 0, 0)
        .unwrap()
        .and_utc()
        .to_rfc3339()
        .into();
    propia["ocupa_agenda"] = true.into();
    eventos.extend([evento, terminado, propia.clone()]);
    fs::write(path, serde_json::to_vec(&d).unwrap()).unwrap();
    assert!(run(&c, &["render"]).status.success());
    let html = fs::read_to_string(base.join("ultimo.html")).unwrap();
    assert!(html.contains("Ausencia ya iniciada"));
    assert!(!html.contains("Evento terminado antes de hoy"));
    let fila = html
        .split("<li>")
        .find(|s| s.contains("Reunión propia sin choque"))
        .unwrap();
    assert!(!fila.contains("Bloqueado"));
    propia["item"]["id"] = "otro-compromiso".into();
    propia["item"]["titulo"] = "Otro compromiso personal".into();
    d["calendario"]["datos"]["eventos"]
        .as_array_mut()
        .unwrap()
        .push(propia);
    fs::write(dir.join("datos.json"), serde_json::to_vec(&d).unwrap()).unwrap();
    assert!(run(&c, &["render"]).status.success());
    let html = fs::read_to_string(base.join("ultimo.html")).unwrap();
    let fila = html
        .split("<li>")
        .find(|s| s.contains("Reunión propia sin choque"))
        .unwrap();
    assert!(fila.contains("Bloqueado"));
}

struct Servidor(Child);
impl Drop for Servidor {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn servidor_acciones_y_lock() {
    let (t, c, port) = preparar();
    assert!(run(&c, &["todo"]).status.success());
    let _server = Servidor(
        Command::new(env!("CARGO_BIN_EXE_diario"))
            .args([
                "--config",
                c.to_str().unwrap(),
                "--demo",
                "--sin-audio",
                "servir",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let url = format!("http://127.0.0.1:{port}");
    let start = Instant::now();
    while ureq::get(&format!("{url}/api/estado")).call().is_err() {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(30));
    }
    assert_eq!(
        ureq::post(&format!("{url}/api/sync"))
            .send_string("{}")
            .unwrap_err()
            .into_response()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        ureq::get(&format!("{url}/api/sync"))
            .call()
            .unwrap_err()
            .into_response()
            .unwrap()
            .status(),
        405
    );
    let base = t.path().join("salida/demo");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(base.join(".corrida.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    assert!(!run(&c, &["todo"]).status.success());
    let response = ureq::post(&format!("{url}/api/sync"))
        .set("Origin", &url)
        .set("X-Diario", "1")
        .send_string("{}")
        .unwrap_err()
        .into_response()
        .unwrap();
    assert_eq!(response.status(), 409);
    FileExt::unlock(&lock).unwrap();
    let ok = ureq::post(&format!("{url}/api/sync"))
        .set("Origin", &url)
        .set("X-Diario", "1")
        .send_string("{}")
        .unwrap();
    assert_eq!(ok.status(), 202);
    let start = Instant::now();
    loop {
        let p: Value = ureq::get(&format!("{url}/api/estado"))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        if p["corriendo"] == false {
            assert!(p["error"].is_null(), "{p}");
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(40));
    }
}

#[test]
fn barrera_herramientas_y_consultas() {
    use std::io::Write;
    let check = |name: &str, input: Value, qs: &str| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_diario"))
            .args([
                "permiso",
                "--permitidas",
                "mcp__claude_ai_Gmail__get_message,mcp__mcp-clickhouse__run_query",
                "--consultas-json",
                qs,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                serde_json::to_string(&serde_json::json!({"tool_name":name,"tool_input":input}))
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap();
        child.wait().unwrap().success()
    };
    assert!(check(
        "mcp__claude_ai_Gmail__get_message",
        serde_json::json!({"messageId":"demo"}),
        "[]"
    ));
    assert!(!check(
        "mcp__claude_ai_Gmail__send_message",
        serde_json::json!({}),
        "[]"
    ));
    assert!(!check(
        "Bash",
        serde_json::json!({"command":"echo test"}),
        "[]"
    ));
    assert!(check(
        "mcp__mcp-clickhouse__run_query",
        serde_json::json!({"query":"SELECT 1 LIMIT 1"}),
        "[\"SELECT 1 LIMIT 1\"]"
    ));
    assert!(!check(
        "mcp__mcp-clickhouse__run_query",
        serde_json::json!({"query":"SELECT 2 LIMIT 1"}),
        "[\"SELECT 1 LIMIT 1\"]"
    ));
}

#[test]
fn lectura_confinada_a_la_sesion() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let results = dir.path().join("tool-results");
    std::fs::create_dir(&results).unwrap();
    let inside = results.join("resultado.json");
    let outside = dir.path().join("privado.txt");
    std::fs::write(&inside, "{}").unwrap();
    std::fs::write(&outside, "privado").unwrap();
    let check = |path: &std::path::Path| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_diario"))
            .args(["permiso", "--permitidas", "Read", "--lectura"])
            .arg(&results)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                serde_json::to_string(
                    &serde_json::json!({"tool_name":"Read","tool_input":{"file_path":path}}),
                )
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        {
            let salida = child.wait_with_output().unwrap();
            (salida.status.code(), salida.stdout)
        }
    };
    let permitido = check(&inside);
    assert_eq!(permitido.0, Some(0));
    let decision: Value = serde_json::from_slice(&permitido.1)
        .expect("La lectura autorizada debe recibir una decisión explícita para headless");
    assert_eq!(
        decision["hookSpecificOutput"]["permissionDecision"],
        "allow"
    );
    assert_eq!(check(&outside).0, Some(2));
    assert_eq!(check(&results.join("../privado.txt")).0, Some(2));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, results.join("escape.json")).unwrap();
        assert_eq!(check(&results.join("escape.json")).0, Some(2));
    }
}

#[test]
fn lectura_mcp_conserva_los_parametros_exactos() {
    use std::io::Write;
    let tool = "mcp__claude_ai_Atlassian__searchJiraIssuesUsingJql";
    let entrada =
        serde_json::json!({"cloudId":"sitio-autorizado","jql":"filter = 11059","maxResults":100});
    let regla = serde_json::json!({"herramienta":tool,"entradas":[entrada.clone()]}).to_string();
    let check = |value: Value| {
        let mut child = Command::new(env!("CARGO_BIN_EXE_diario"))
            .args(["permiso", "--permitidas", tool, "--entrada-json", &regla])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(
                serde_json::json!({"tool_name":tool,"tool_input":value})
                    .to_string()
                    .as_bytes(),
            )
            .unwrap();
        child.wait().unwrap().success()
    };
    assert!(check(entrada.clone()));
    let mut otra = entrada.clone();
    otra["cloudId"] = serde_json::json!("otro-sitio");
    assert!(!check(otra));
    let mut otra = entrada.clone();
    otra["jql"] = serde_json::json!("project = CORE");
    assert!(!check(otra));
    let mut otra = entrada;
    otra["expand"] = serde_json::json!("todo");
    assert!(!check(otra));
}

#[test]
fn equipo_solo_edicion_conserva_las_metricas_guardadas() {
    let (t, c, _) = preparar();
    assert!(run(&c, &["todo"]).status.success());
    let base = t.path().join("salida/demo/equipo");
    let dir = fs::read_dir(&base)
        .unwrap()
        .filter_map(Result::ok)
        .find(|p| p.path().join("reporte.json").is_file())
        .unwrap()
        .path();
    let path = dir.join("reporte.json");
    let mut reporte: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    reporte["vistas"][0]["cobertura"] = serde_json::json!(3);
    reporte["resumen"]["datos"] = Value::Null;
    let vistas = reporte["vistas"].clone();
    fs::write(&path, serde_json::to_vec(&reporte).unwrap()).unwrap();
    let salud = fs::read(dir.join("salud.json")).unwrap();
    let result = run(&c, &["equipo", "--solo-edicion"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let nuevo: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(nuevo["vistas"], vistas);
    assert!(nuevo["resumen"]["datos"]
        .as_array()
        .is_some_and(|xs| (3..=5).contains(&xs.len())));
    assert_eq!(fs::read(dir.join("salud.json")).unwrap(), salud);
}
