use crate::{
    archivos,
    config::Config,
    modelo::{Datos, Dolar},
};
use anyhow::{bail, Context, Result};
use chrono::{Duration, NaiveDate};
use regex::Regex;
use serde_json::{json, Value};

pub const BCB: &str = "https://www.bcb.gob.bo/librerias/indicadores/otras/ultimo.php";
pub const BINANCE: &str = "https://p2p.binance.com/bapi/c2c/v2/friendly/c2c/adv/search";

pub fn parsear_bcb(html: &str) -> Result<(f64, NaiveDate)> {
    let precio =
        Regex::new(r"(?is)<td[^>]*>\s*USD\s*</td>\s*<td[^>]*>\s*([0-9]+[.,][0-9]+)\s*</td>")?
            .captures(html)
            .context("No se reconoció el TCO USD del BCB")?[1]
            .replace(',', ".")
            .parse::<f64>()?;
    let re = Regex::new(r"(?i)(\d{1,2})\s+de\s+([a-z]+)\s+(?:de\s+)?(20\d{2})")?;
    let caps = re.captures(html).context("Falta fecha de cotización BCB")?;
    let meses = [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ];
    let mes = meses
        .iter()
        .position(|m| *m == caps[2].to_lowercase())
        .context("Mes BCB inválido")? as u32
        + 1;
    let fecha = NaiveDate::from_ymd_opt(caps[3].parse()?, mes, caps[1].parse()?)
        .context("Fecha BCB inválida")?;
    if !precio.is_finite() || precio <= 0.0 {
        bail!("TCO inválido");
    }
    Ok((precio, fecha))
}
pub fn mediana(v: &Value) -> Result<(f64, usize)> {
    if v["code"] != "000000" {
        bail!("Binance no devolvió cotizaciones");
    }
    let mut ps: Vec<f64> = v["data"]
        .as_array()
        .context("Falta lista de ofertas Binance")?
        .iter()
        .filter_map(|a| a["adv"]["price"].as_str()?.parse::<f64>().ok())
        .filter(|n| n.is_finite() && *n > 0.0)
        .collect();
    if ps.len() < 3 {
        bail!("Se requieren al menos tres ofertas USDT/BOB");
    }
    ps.sort_by(f64::total_cmp);
    let n = ps.len();
    let p = if n.is_multiple_of(2) {
        (ps[n / 2 - 1] + ps[n / 2]) / 2.0
    } else {
        ps[n / 2]
    };
    Ok((p, n))
}
pub fn obtener(c: &Config, fecha: NaiveDate) -> Result<Dolar> {
    let html = super::http().get(BCB).call()?.into_string()?;
    let (oficial, oficial_fecha) = parsear_bcb(&html)?;
    if oficial_fecha > fecha || fecha.signed_duration_since(oficial_fecha).num_days() > 7 {
        bail!("Cotización BCB fuera de fecha: {oficial_fecha}");
    }
    let v:Value=super::http().post(BINANCE).send_json(json!({"asset":"USDT","fiat":"BOB","tradeType":"BUY","page":1,"rows":10,"payTypes":[],"publisherType":"merchant"}))?.into_json()?;
    let (paralelo, muestras) = mediana(&v)?;
    let cambio = |dias: i64| -> Option<f64> {
        let d: Datos = archivos::leer(
            &archivos::dia(&c.salida, fecha - Duration::days(dias)).join("datos.json"),
        )
        .ok()?;
        let p = d.dolar.datos?.paralelo;
        (p > 0.0).then_some((paralelo / p - 1.0) * 100.0)
    };
    Ok(Dolar {
        oficial,
        oficial_fecha,
        paralelo,
        muestras,
        brecha: (paralelo / oficial - 1.0) * 100.0,
        cambio_diario: cambio(1),
        cambio_cinco_dias: cambio(5),
        oficial_url: BCB.into(),
        paralelo_url: "https://p2p.binance.com/es/trade/all-payments/USDT?fiat=BOB".into(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn formato_bcb() {
        let s = "19 de Noviembre 2026<td>USD</td><td class=\"numero\">11.68</td>";
        let (p, d) = parsear_bcb(s).unwrap();
        assert_eq!(p, 11.68);
        assert_eq!(d.to_string(), "2026-11-19");
        assert!(parsear_bcb("11.68").is_err());
    }
    #[test]
    fn no_primera_oferta() {
        let v = json!({"code":"000000","data":[{"adv":{"price":"9"}},{"adv":{"price":"10"}},{"adv":{"price":"99"}},{"adv":{"price":"11"}}]});
        assert_eq!(mediana(&v).unwrap(), (10.5, 4));
    }
}
