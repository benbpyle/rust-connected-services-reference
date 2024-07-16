use axum::{
    extract::{Query, State},
    http::{HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use opentelemetry::global;
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry_datadog::{new_pipeline, ApiVersion};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use reqwest::{Client, Error};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::ParseBoolError, time::Duration};
use tracing::{instrument, Span};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Registry};

#[derive(Serialize, Deserialize, Debug)]
struct ExternalModel {
    key_one: String,
    key_two: String,
    key_time: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug)]
struct ServiceAModel {
    key_one: String,
    key_two: String,
}

#[derive(Serialize, Deserialize, Debug)]
struct ServiceCModel {
    key_time: DateTime<Utc>,
}

#[derive(Deserialize, Debug)]
struct Prefix {
    name: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HealthCheck {
    status: String,
}

#[derive(Clone, Debug)]
struct AppState {
    http_client: Client,
}

#[tokio::main]
async fn main() {
    global::set_text_map_propagator(TraceContextPropagator::new());

    let app_state = AppState {
        http_client: Client::new(),
    };

    let tracing_enabled =
        std::env::var("DD_TRACING_ENABLED").expect("DD_TRACING_ENABLED is required");

    let use_tracing: Result<bool, ParseBoolError> = tracing_enabled.parse();
    let flag = if let Ok(b) = use_tracing { b } else { false };

    let fmt_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_target(false)
        .without_time();

    if flag {
        let agent_address = std::env::var("AGENT_ADDRESS").expect("AGENT_ADDRESS is required");
        let tracer = match new_pipeline()
            .with_service_name("service-b")
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

    let bind_address = std::env::var("BIND_ADDRESS").expect("BIND_ADDRESS is required");
    let app = Router::new()
        .route("/", get(handler))
        .route("/health", get(health))
        .with_state(app_state);
    let listener = tokio::net::TcpListener::bind(bind_address.clone())
        .await
        .unwrap();
    tracing::info!("Up and running ... listening on {}", bind_address);
    axum::serve(listener, app).await.unwrap();
}

#[tracing::instrument(name = "GET /")]
async fn handler(
    State(state): State<AppState>,
    Query(q): Query<Prefix>,
) -> Result<impl IntoResponse, StatusCode> {
    let service_a_model_response = get_service_a(&state.http_client, q).await?;
    let service_c_model_response = get_service_c(&state.http_client).await?;
    let external_model = ExternalModel {
        key_one: service_a_model_response.key_one,
        key_two: service_a_model_response.key_two,
        key_time: service_c_model_response.key_time,
    };
    Ok(Json(external_model))
}

#[instrument(name = "http-service-c")]
async fn get_service_c(client: &Client) -> Result<ServiceCModel, StatusCode> {
    let service_c_host: String = std::env::var("SERVICE_C_URL").expect("SERVICE_C_URL Must be Set");
    let url = format!("{}/time", service_c_host);

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

    let response = client.get(url.as_str()).headers(headers).send().await;
    match response {
        Ok(r) => {
            if r.status().is_success() {
                let j: Result<ServiceCModel, Error> = r.json().await;
                match j {
                    Ok(m) => Ok(m),
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

#[instrument(name = "http-service-a")]
async fn get_service_a(client: &Client, q: Prefix) -> Result<ServiceAModel, StatusCode> {
    let service_a_host: String = std::env::var("SERVICE_A_URL").expect("SERVICE_A_URL Must be Set");

    let prefix: String;
    let passed_value = &q.name;

    if let Some(s) = passed_value {
        prefix = String::from(s.as_str());
    } else {
        prefix = String::from("Unknown");
    }

    let url = format!("{}/route?p={}", service_a_host, prefix);
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

    let response = client.get(url.as_str()).headers(headers).send().await;
    tracing::info!("(Response)={:?}", response);
    match response {
        Ok(r) => {
            if r.status().is_success() {
                let j: Result<ServiceAModel, Error> = r.json().await;
                match j {
                    Ok(m) => Ok(m),
                    Err(e) => {
                        tracing::error!("Error parsing: {}", e);
                        Err(StatusCode::BAD_REQUEST)
                    }
                }
            } else if r.status() == StatusCode::GATEWAY_TIMEOUT {
                let model = ServiceAModel {
                    key_one: "Timed out".to_string(),
                    key_two: "Timed out".to_string(),
                };
                Ok(model)
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

#[cfg(test)]
mod tests {
    #[test]
    fn fake_1() {
        let s = "one";
        assert_eq!("one", s);
    }
}
