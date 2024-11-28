use axum::{
    extract::{Json, State},
    routing::post,
    Router,
    response::{IntoResponse, Response},
};
use baselard::{
    component::{Component, ComponentRegistry, Data, DataType},
    components::{adder::Adder, payload_transformer::PayloadTransformer},
    dag::{DAGConfig, DAGError, DAG, DAGIR},
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use baselard::cache::DAGCache;

#[derive(Debug)]
struct Multiplier {
    value: f64,
}

impl Component for Multiplier {
    fn configure(config: serde_json::Value) -> Self {
        let multiplier = config["multiplier"].as_f64().unwrap_or(0.0);
        Self { value: multiplier }
    }

    fn execute(&self, input: Data) -> Result<Data, DAGError> {
        let input_value = match input {
            Data::Null => 0.0,
            Data::Integer(n) => n as f64,
            Data::List(list) => list
                .iter()
                .filter_map(|v| v.as_integer())
                .map(|n| n as f64)
                .sum(),
            _ => {
                return Err(DAGError::TypeSystemFailure {
                    component: "Multiplier".to_string(),
                    expected: self.input_type(),
                    received: input.get_type(),
                })
            }
        };

        Ok(Data::Integer((input_value * self.value) as i32))
    }

    fn input_type(&self) -> DataType {
        DataType::Union(vec![
            DataType::Null,
            DataType::Integer,
            DataType::List(Box::new(DataType::Integer)),
        ])
    }

    fn output_type(&self) -> DataType {
        DataType::Integer
    }
}

struct AppState {
    registry: Arc<ComponentRegistry>,
    cache: Arc<DAGCache>,
}

async fn execute_dag(
    State(state): State<Arc<AppState>>,
    Json(dag_config): Json<Value>,
) -> Response {
    let start = Instant::now();

    let result = match DAGIR::from_json(dag_config) {
        Ok(ir) => match DAG::from_ir(ir, &state.registry, DAGConfig::default(), Some(Arc::clone(&state.cache))) {
            Ok(dag) => dag.execute(None).await,
            Err(e) => Err(DAGError::InvalidConfiguration(e.to_string())),
        },
        Err(e) => Err(DAGError::InvalidConfiguration(e)),
    };

    let elapsed = start.elapsed().as_millis();

    match result {
        Ok(outputs) => Json(json!({
            "success": true,
            "results": outputs,
            "took_ms": elapsed
        })).into_response(),
        Err(err) => Json(json!({
            "success": false,
            "error": format!("{:?}", err),
            "took_ms": elapsed
        })).into_response(),
    }
}

#[tokio::main]
async fn main() {
    let mut registry = ComponentRegistry::new();
    registry.register::<Adder>("Adder");
    registry.register::<Multiplier>("Multiplier");
    registry.register::<PayloadTransformer>("PayloadTransformer");

    let cache = DAGCache::new(
        Some("/tmp/axum_dag_history.jsonl"),
        10_000,
    );

    let state = Arc::new(AppState {
        registry: Arc::new(registry),
        cache: Arc::new(cache),
    });

    let app = Router::new()
        .route("/execute", post(execute_dag))
        .with_state(state);

    println!("Server running on http://localhost:3000");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();

    axum::serve(listener, app)
        .await
        .unwrap();
}