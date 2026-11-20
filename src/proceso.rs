use anyhow::{bail, Context, Result};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub fn ejecutar(programa: &str, args: &[String], entrada: &str, timeout: u64) -> Result<String> {
    let mut cmd = Command::new(programa);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd
        .spawn()
        .with_context(|| format!("No se pudo iniciar {programa}"))?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || {
        let mut b = Vec::new();
        let r = stdout
            .by_ref()
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut b);
        (r, b)
    });
    let err = thread::spawn(move || {
        let mut b = Vec::new();
        let r = stderr.by_ref().take(1024 * 1024).read_to_end(&mut b);
        (r, b)
    });
    if let Some(mut stdin) = child.stdin.take() {
        let bytes = entrada.as_bytes().to_vec();
        thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait()? {
            break s;
        }
        if start.elapsed() > Duration::from_secs(timeout) {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            bail!("{programa}: tiempo agotado ({timeout} s)");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let (r, b) = out
        .join()
        .map_err(|_| anyhow::anyhow!("Lectura interrumpida"))?;
    r?;
    let _ = err.join();
    // stderr puede contener configuración o credenciales de MCP: no se propaga.
    if !status.success() {
        if programa == "gh" {
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&b) {
                if let Some(message) = v["errors"][0]["message"].as_str().or(v["message"].as_str())
                {
                    bail!("GitHub API: {message}");
                }
            }
        }
        bail!(
            "{programa} terminó con código {:?}; revisar autenticación/conexiones",
            status.code()
        );
    }
    if b.len() > 16 * 1024 * 1024 {
        bail!("Respuesta demasiado grande; recolección incompleta");
    }
    String::from_utf8(b).context("Respuesta no UTF-8")
}
