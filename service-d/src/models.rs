use core::f64;

use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherResponse {
    city: String,
    state: String,
    celcius: f64,
    farenheight: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherApiResponse {
    location: WeatherApiLocationResponse,
    current: WeatherApiCurrentResponse,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherApiLocationResponse {
    name: String,
    region: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WeatherApiCurrentResponse {
    temp_c: f64,
    temp_f: f64,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Prefix {
    pub zip: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HealthCheck {
    pub status: String,
}

#[derive(Clone, Debug)]
pub struct AppState {
    pub has_apm: bool,
    pub http_client: Client,
}

impl From<WeatherApiResponse> for WeatherResponse {
    fn from(r: WeatherApiResponse) -> Self {
        WeatherResponse {
            celcius: r.current.temp_c,
            farenheight: r.current.temp_f,
            city: r.location.name,
            state: r.location.region,
        }
    }
}
