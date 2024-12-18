use baselard::component::Data;
use baselard::component::Registry;
use baselard::components::payload_transformer::PayloadTransformer;
use baselard::dag::{DAGSettings, DAG};
use baselard::dagir::DAGIR;
use serde_json::json;

fn setup_test_registry() -> Registry {
    let mut registry = Registry::new();
    registry.register::<PayloadTransformer>("PayloadTransformer");
    registry
}

#[tokio::test]
async fn test_basic_transformation() {
    let registry = setup_test_registry();
    let json_config = json!({
        "alias": "basic_transformation_test",
        "nodes": [{
            "id": "transform1",
            "component_type": "PayloadTransformer",
            "config": {
            "transformation_expression": ".message",
            "validation_data": {
                "input": {
                        "message": "test message",
                        "extra": "ignored"
                    },
                    "expected_output": "test message"
                }
            },
            "inputs": {
                "message": "Hello, World!",
                "extra": "ignored"
            }
        }]
    });

    let dag = DAGIR::from_json(&json_config)
        .and_then(|ir| DAG::from_ir(&ir, &registry, DAGSettings::default(), None))
        .expect("Valid DAG");

    let results = dag.execute(None).await.expect("Execution success");
    assert_eq!(
        results.get("transform1"),
        Some(&Data::Json(json!("Hello, World!")))
    );
}

#[tokio::test]
async fn test_invalid_jq_expression() {
    let registry = setup_test_registry();
    let json_config = json!({
        "alias": "invalid_jq_expression_test",
        "nodes": [{
            "id": "transform1",
            "component_type": "PayloadTransformer",
            "config": {
                "transformation_expression": "invalid[expression",
                "validation_data": {
                    "input": {"test": "data"},
                    "expected_output": "data"
                }
            },
            "inputs": {"test": "data"}
        }]
    });

    let result = DAGIR::from_json(&json_config)
        .and_then(|ir| DAG::from_ir(&ir, &registry, DAGSettings::default(), None));

    assert!(
        result.is_err(),
        "Invalid JQ expression should fail at configuration"
    );
    assert!(
        matches!(
            result,
            Err(e) if e.to_string().contains("JQ program validation failed")
        ),
        "Error should mention JQ validation failure"
    );
}

#[tokio::test]
async fn test_chained_transformations() {
    let registry = setup_test_registry();
    let json_config = json!({
        "alias": "chained_transformations_test",
        "nodes": [{
            "id": "transform1",
            "component_type": "PayloadTransformer",
            "config": {
                "transformation_expression": "{message: .message, count: (.count + 1)}",
                "validation_data": {
                    "input": {
                        "message": "test",
                        "count": 0
                    },
                    "expected_output": {
                        "message": "test",
                        "count": 1
                    }
                }
            },
            "inputs": {
                "message": "Hello",
                "count": 0
            }
        },
        {
            "id": "transform2",
            "component_type": "PayloadTransformer",
            "config": {
                "transformation_expression": ".message + \" World!\"",
                "validation_data": {
                    "input": {
                        "message": "test"
                    },
                    "expected_output": "test World!"
                }
            },
            "depends_on": ["transform1"]
        }]
    });

    let dag = DAGIR::from_json(&json_config)
        .and_then(|ir| DAG::from_ir(&ir, &registry, DAGSettings::default(), None))
        .expect("Valid DAG");

    let results = dag.execute(None).await.expect("Execution success");

    assert_eq!(
        results.get("transform1"),
        Some(&Data::Json(json!({"message": "Hello", "count": 1})))
    );

    assert_eq!(
        results.get("transform2"),
        Some(&Data::Json(json!("Hello World!")))
    );
}

#[tokio::test]
async fn test_non_json_input() {
    let registry = setup_test_registry();
    let json_config = json!({
        "alias": "non_json_input_test",
        "nodes": [{
            "id": "transform1",
            "component_type": "PayloadTransformer",
            "config": {
                "transformation_expression": ".",
                "validation_data": {
                "input": 42,
                "expected_output": 42
                }
            },
            "inputs": 42  // Integer instead of JSON
        }]
    });

    let result = DAGIR::from_json(&json_config)
        .and_then(|ir| DAG::from_ir(&ir, &registry, DAGSettings::default(), None));

    assert!(result.is_err(), "Non-JSON input should fail DAG validation");
    assert!(
        matches!(
            result,
            Err(e) if e.to_string().contains("Expected Json, got Integer")
        ),
        "Error should mention type mismatch"
    );
}

#[tokio::test]
async fn test_default_identity_transform() {
    let registry = setup_test_registry();
    let json_config = json!({
        "alias": "default_identity_transform_test",
        "nodes": [{
            "id": "transform1",
            "component_type": "PayloadTransformer",
            "config": {
            "validation_data": {
                "input": {"test": "data"},
                "expected_output": {"test": "data"}
            }
        },  // No expression provided, should default to "."
            "inputs": {"test": "data"}
        }]
    });

    let dag = DAGIR::from_json(&json_config)
        .and_then(|ir| DAG::from_ir(&ir, &registry, DAGSettings::default(), None))
        .expect("Valid DAG");

    let results = dag.execute(None).await.expect("Execution success");
    assert_eq!(
        results.get("transform1"),
        Some(&Data::Json(json!({"test": "data"})))
    );
}
