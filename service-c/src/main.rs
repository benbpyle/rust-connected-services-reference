use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use chrono::{DateTime, Utc};
use opentelemetry::propagation::TextMapPropagator;
use opentelemetry_datadog::{new_pipeline, ApiVersion};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::ParseBoolError};
use tracing::instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Registry};

#[derive(Serialize, Deserialize, Debug)]
struct ExternalModel {
    key_time: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HealthCheck {
    status: String,
}

#[derive(Clone, Debug)]
struct AppState {
    has_apm: bool,
}

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
            .with_service_name("service-c")
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

    let app_state = AppState { has_apm: flag };
    let bind_address = std::env::var("BIND_ADDRESS").expect("BIND_ADDRESS is required");
    let app = Router::new()
        .route("/time", get(handler))
        .route("/", get(health))
        .with_state(app_state);
    let listener = tokio::net::TcpListener::bind(bind_address.clone())
        .await
        .unwrap();
    tracing::info!("Up and running ... listening on {}", bind_address);
    axum::serve(listener, app).await.unwrap();
}

#[instrument(name = "GET /time")]
async fn handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, axum::http::StatusCode> {
    let m = ExternalModel {
        key_time: Utc::now(),
    };

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
    tracing::info!("(Request)={:?}|(Headers)={:?}", m, headers);
    Ok(Json(m))
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
