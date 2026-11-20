use crate::{config::Config, modelo::Clima};
use anyhow::{Context, Result};
use serde_json::Value;

pub fn obtener(c: &Config) -> Result<Clima> {
    let url=format!("https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m&daily=temperature_2m_max,temperature_2m_min,precipitation_probability_max&timezone=America%2FLa_Paz&forecast_days=1",c.latitud,c.longitud);
    let v: Value = super::http().get(&url).call()?.into_json()?;
    let num = |x: &Value| x.as_f64().context("Respuesta de clima incompleta");
    Ok(Clima {
        actual: num(&v["current"]["temperature_2m"])?,
        maxima: num(&v["daily"]["temperature_2m_max"][0])?,
        minima: num(&v["daily"]["temperature_2m_min"][0])?,
        lluvia: num(&v["daily"]["precipitation_probability_max"][0])?,
        url,
    })
}
