use baselard::cache::DAGCache;
use baselard::component::ComponentRegistry;
use baselard::component::{Component, Data, DataType};
use baselard::components::adder::Adder;
use baselard::dag::{DAGConfig, DAGError, DAG, DAGIR};
use serde_json::json;
use std::sync::Arc;

#[derive(Debug)]
struct Multiplier {
    value: f64,
}

impl Component for Multiplier {
    fn configure(config: serde_json::Value) -> Self {
        let multiplier = config["multiplier"].as_f64().unwrap();
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

fn setup_test_registry() -> ComponentRegistry {
    let mut registry = ComponentRegistry::new();
    registry.register::<Adder>("Adder");
    registry.register::<Multiplier>("Multiplier");
    registry
}

#[tokio::test]
async fn test_basic_multiplication() {
    let registry = setup_test_registry();
    let json_config = json!([{
        "id": "mult1",
        "component_type": "Multiplier",
        "config": { "multiplier": 2.5 },
        "inputs": 10
    }]);

    let dag = DAGIR::from_json(json_config)
        .and_then(|ir| DAG::from_ir(ir, &registry, DAGConfig::default(), None))
        .expect("Valid DAG");

    let results = dag.execute(None).await.expect("Execution success");
    assert_eq!(results.get("mult1"), Some(&Data::Integer(25)));
}

#[tokio::test]
async fn test_chained_operations() {
    let registry = setup_test_registry();
    let json_config = json!([
        {
            "id": "adder_1",
            "component_type": "Adder",
            "config": { "value": 5 },
            "inputs": 10
        },
        {
            "id": "mult_1",
            "component_type": "Multiplier",
            "config": { "multiplier": 2.0 },
            "depends_on": ["adder_1"]
        },
        {
            "id": "adder_2",
            "component_type": "Adder",
            "config": { "value": 3 },
            "depends_on": ["mult_1"]
        }
    ]);

    let dag = DAGIR::from_json(json_config)
        .and_then(|ir| DAG::from_ir(ir, &registry, DAGConfig::default(), None))
        .expect("Valid DAG");

    let results = dag.execute(None).await.expect("Execution success");

    assert_eq!(results.get("adder_1"), Some(&Data::Integer(15)));

    assert_eq!(results.get("mult_1"), Some(&Data::Integer(30)));

    assert_eq!(results.get("adder_2"), Some(&Data::Integer(33)));
}

#[tokio::test]
async fn test_error_handling() {
    let registry = setup_test_registry();

    let invalid_config = json!([{
        "id": "mult1",
        "component_type": "Multiplier",
        "config": { "wrong_field": 2.5 },
        "inputs": 10
    }]);

    let result = DAGIR::from_json(invalid_config)
        .and_then(|ir| DAG::from_ir(ir, &registry, DAGConfig::default(), None));
    assert!(matches!(result, Err(_)));

    let invalid_input = json!([{
        "id": "mult1",
        "component_type": "Multiplier",
        "config": { "multiplier": 2.5 },
        "inputs": "not a number"
    }]);

    let result = DAGIR::from_json(invalid_input)
        .and_then(|ir| DAG::from_ir(ir, &registry, DAGConfig::default(), None));
    assert!(matches!(result, Err(_)));
}

#[tokio::test]
async fn test_caching_and_replay() {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let history_file = temp_dir.path().join("multiplier_history.jsonl");
    let cache = Arc::new(DAGCache::new(Some(history_file), 10_000));

    let json_config = json!([
        {
            "id": "mult1",
            "component_type": "Multiplier",
            "config": { "multiplier": 3.0 },
            "inputs": 5
        }
    ]);

    let registry = setup_test_registry();
    let dag = DAGIR::from_json(json_config)
        .and_then(|ir| {
            DAG::from_ir(
                ir,
                &registry,
                DAGConfig::default(),
                Some(Arc::clone(&cache)),
            )
        })
        .expect("Valid DAG");

    let request_id = "mult-test-1".to_string();
    let original_results = dag
        .execute(Some(request_id.clone()))
        .await
        .expect("Execution success");

    assert_eq!(original_results.get("mult1"), Some(&Data::Integer(15)));

    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    let replayed_results = dag.replay(&request_id).await.expect("Replay success");

    assert_eq!(original_results, replayed_results);
}

#[tokio::test]
async fn test_default_input() {
    let registry = setup_test_registry();
    let json_config = json!([{
        "id": "mult1",
        "component_type": "Multiplier",
        "config": { "multiplier": 2.0 }

    }]);

    let dag = DAGIR::from_json(json_config)
        .and_then(|ir| DAG::from_ir(ir, &registry, DAGConfig::default(), None))
        .expect("Valid DAG");

    let results = dag.execute(None).await.expect("Execution success");
    assert_eq!(results.get("mult1"), Some(&Data::Integer(0)));
}
