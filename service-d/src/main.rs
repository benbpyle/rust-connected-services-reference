use axum::{
    extract::{Query, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use models::{HealthCheck, Prefix};
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry_datadog::{new_pipeline, ApiVersion};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use reqwest::{Client, Error};
use std::{collections::HashMap, str::ParseBoolError};
use tracing::{instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Registry};

use crate::models::WeatherApiResponse;
use crate::models::{AppState, WeatherResponse};
mod models;

#[tokio::main]
async fn main() {
    let tracing_enabled =
        std::env::var("DD_TRACING_ENABLED").expect("DD_TRACING_ENABLED is required");

    let use_tracing: Result<bool, ParseBoolError> = tracing_enabled.parse();
    let flag = if let Ok(b) = use_tracing { b } else { false };

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .without_time();

    if flag {
        let agent_address = std::env::var("AGENT_ADDRESS").expect("AGENT_ADDRESS is required");
        let tracer = match new_pipeline()
            .with_service_name("service-d")
            .with_agent_endpoint(format!("http://{}:8126", agent_address))
            .with_api_version(ApiVersion::Version05)
            .install_batch(opentelemetry_sdk::runtime::Tokio)
        {
            Ok(a) => a,
            Err(e) => {
                panic!("error starting! {}", e);
            }
        };
        let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        Registry::default()
            .with(fmt_layer)
            .with(telemetry_layer)
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    } else {
        Registry::default()
            .with(fmt_layer)
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    }

    let app_state = AppState {
        has_apm: flag,
        http_client: Client::new(),
    };

    let address = std::env::var("BIND_ADDRESS").expect("BIND_ADDRESS is required");
    let app = Router::new()
        .route("/weather", get(handler))
        .route("/health", get(health))
        .with_state(app_state);
    let listener = tokio::net::TcpListener::bind(address.clone())
        .await
        .unwrap();
    tracing::info!("Up and running ... listening on {}", address);
    axum::serve(listener, app).await.unwrap();
}

#[instrument(name = "GET /weather")]
async fn handler(
    State(state): State<AppState>,
    query: Query<Prefix>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, StatusCode> {
    if state.has_apm {
        let mut fields: HashMap<String, String> = HashMap::new();
        fields.insert(
            "traceparent".to_string(),
            String::from(headers.get("traceparent").unwrap().to_str().unwrap()),
        );

        let propagator = TraceContextPropagator::new();
        let context = propagator.extract(&fields);
        let span = tracing::Span::current();
        span.set_parent(context);
    }
    let prefix: String;
    let passed_value = &query.zip;

    if let Some(s) = passed_value {
        prefix = String::from(s.as_str());
    } else {
        prefix = String::from("76262");
    }

    tracing::info!("(Request)={}", prefix);

    let weather_api_host: String =
        std::env::var("WEATHER_API_URL").expect("WEATHER_API_URL Must be Set");
    let weather_api_key: String =
        std::env::var("WEATHER_API_KEY").expect("WEATHER_API_KEY Must be set");

    let url = format!(
        "{}/current.json?q={}&key={}",
        weather_api_host, prefix, weather_api_key
    );
    let ctx = Span::current().context();
    let propagator = TraceContextPropagator::new();
    let mut fields = HashMap::new();

    propagator.inject_context(&ctx, &mut fields);
    let headers = fields
        .into_iter()
        .map(|(k, v)| {
            (
                HeaderName::try_from(k).unwrap(),
                HeaderValue::try_from(v).unwrap(),
            )
        })
        .collect();
    tracing::info!("(Request)={}", url.as_str());

    let response = state
        .http_client
        .get(url.as_str())
        .headers(headers)
        .send()
        .await;

    match response {
        Ok(r) => {
            if r.status().is_success() {
                let j: Result<WeatherApiResponse, Error> = r.json().await;
                match j {
                    Ok(m) => Ok(Json(WeatherResponse::from(m))),
                    Err(e) => {
                        tracing::error!("Error parsing: {}", e);
                        Err(StatusCode::BAD_REQUEST)
                    }
                }
            } else {
                tracing::error!("Bad request={:?}", r.status());
                Err(StatusCode::BAD_REQUEST)
            }
        }
        Err(e) => {
            tracing::error!("Error requesting: {}", e);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

async fn health() -> Result<impl IntoResponse, StatusCode> {
    let healthy = HealthCheck {
        status: String::from("Healthy"),
    };

    Ok(Json(healthy))
}
