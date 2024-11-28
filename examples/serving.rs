use axum::{
    extract::{Json, State},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};

use baselard::cache::Cache;
use baselard::dag_visualizer::TreeView;

use baselard::{
    component::{Component, Data, DataType, Registry},
    components::{adder::Adder, payload_transformer::PayloadTransformer},
    dag::{DAGConfig, DAGError, DAG, DAGIR},
};
use serde_json::{json, Value};
use tracing::warn;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug)]
struct Multiplier {
    value: f64,
}

impl Component for Multiplier {
    fn configure(config: serde_json::Value) -> Self {
        let multiplier = config["multiplier"].as_f64().unwrap_or(0.0);
        Self { value: multiplier }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn execute(&self, input: Data) -> Result<Data, DAGError> {
        let input_value = match input {
            Data::Null => 0.0,
            Data::Integer(n) => f64::from(n),
            Data::List(list) => list
                .iter()
                .filter_map(baselard::component::Data::as_integer)
                .map(f64::from)
                .sum(),
            Data::Json(value) => {
                println!("[Experimental] Multiplier received JSON value: {value:?}");
                if let Some(num) = value.as_i64() {
                    num as f64
                } else if let Some(num) = value.as_f64() {
                    num
                } else if let Some(list) = value.as_array() {
                    list.iter().filter_map(serde_json::Value::as_f64).sum()
                } else if let Some(text) = value.as_str() {
                    text.parse::<f64>().unwrap_or(0.0)
                } else {
                    warn!("Multiplier received unparseable JSON value: {value:?}");
                    0.0
                }
            },
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
            DataType::Json, // This allows for payload transformations
        ])
    }

    fn output_type(&self) -> DataType {
        DataType::Integer
    }
}

struct AppState {
    registry: Arc<Registry>,
    cache: Arc<Cache>,
}

async fn execute_dag(
    State(state): State<Arc<AppState>>,
    headers: axum::http::HeaderMap,
    Json(dag_config): Json<Value>,
) -> Response {
    let start = Instant::now();

    let use_cache = !headers
        .get(axum::http::header::CACHE_CONTROL)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| s.to_lowercase().contains("no-cache"));

    let result = match DAGIR::from_json(dag_config) {
        Ok(ir) => {
            let tree = ir.build_tree(TreeView::Dependency);
            let mut output = String::new();
            let _ = ascii_tree::write_tree(&mut output, &tree);
            println!("{output}");

            let mut dag_config = DAGConfig::default();
            if !use_cache {
                println!("Disabling memory cache");
                dag_config.enable_memory_cache = false;
            }

            match DAG::from_ir(
                ir,
                &state.registry,
                dag_config,
                Some(Arc::clone(&state.cache)),
            ) {
                Ok(dag) => dag.execute(None).await,
                Err(e) => Err(DAGError::InvalidConfiguration(e.to_string())),
            }
        }
        Err(e) => Err(DAGError::InvalidConfiguration(e)),
    };

    let elapsed = start.elapsed().as_millis();

    match result {
        Ok(outputs) => Json(json!({
            "success": true,
            "results": outputs,
            "took_ms": elapsed,
            "cache_enabled": use_cache
        }))
        .into_response(),
        Err(err) => Json(json!({
            "success": false,
            "error": format!("{:?}", err),
            "took_ms": elapsed,
            "cache_enabled": use_cache
        }))
        .into_response(),
    }
}

#[tokio::main]
async fn main() {
    let mut registry = Registry::new();
    registry.register::<Adder>("Adder");
    registry.register::<Multiplier>("Multiplier");
    registry.register::<PayloadTransformer>("PayloadTransformer");

    let cache = Cache::new(Some("/tmp/axum_dag_history.jsonl"), 10_000);

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

    axum::serve(listener, app).await.unwrap();
}
