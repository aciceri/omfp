//! JSON Schemas of method params and results, for deriving agent tools. Their whole text is pinned once, as the
//! TypeScript types generated from them (`ui/packages/protocol/src/generated.ts`, checked by xtask's staleness test).

use omfp_protocol::{METHODS, method_schemas, type_schemas};
use serde_json::{Value, json};

// spec: AGT-007
#[test]
fn every_method_has_params_and_result_schemas() {
    let schemas = method_schemas();
    let object = schemas.as_object().expect("schemas keyed by method name");
    let mut names: Vec<&str> = object.keys().map(String::as_str).collect();
    let mut expected = METHODS.to_vec();
    names.sort_unstable();
    expected.sort_unstable();
    assert_eq!(names, expected);
    for (name, entry) in object {
        assert!(entry["params"].is_object(), "{name} params");
        assert!(entry["result"].is_object(), "{name} result");
    }
}

// spec: AGT-007
#[test]
fn params_schemas_describe_the_wire_fields() {
    let schemas = method_schemas();
    let prompt = &schemas["thread.prompt"]["params"];
    assert_eq!(prompt["type"], json!("object"));
    let mut required: Vec<&str> = prompt["required"]
        .as_array()
        .expect("required list")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    required.sort_unstable();
    assert_eq!(required, ["text", "threadId"]);
    assert!(prompt["properties"]["behavior"].is_object());

    let subscribe = &schemas["subscribe"]["params"];
    assert!(subscribe["properties"]["since"].is_object());
}

// spec: AGT-007, DMN-020
#[test]
fn thread_creation_schema_requires_an_explicit_workspace_without_a_default() {
    let schemas = method_schemas();
    let params = &schemas["threads.create"]["params"];
    assert!(
        params["required"]
            .as_array()
            .unwrap()
            .contains(&json!("workspace"))
    );
    assert!(params["properties"]["workspace"].get("default").is_none());
}

// spec: AGT-007, AGT-042
#[test]
fn model_discovery_describes_the_selector_fields_for_both_consumers() {
    let schemas = method_schemas();
    for method in ["auth.models", "thread.models"] {
        let result = &schemas[method]["result"];
        assert_eq!(result["type"], "object");
        assert_eq!(result["required"], json!(["models"]));
        let models = &result["properties"]["models"];
        assert_eq!(models["type"], "array");
        let reference = models["items"]["$ref"].as_str().unwrap();
        let entry = result
            .pointer(reference.strip_prefix('#').unwrap())
            .unwrap();
        assert_eq!(entry["type"], "object");
        for field in ["provider", "id", "name"] {
            assert_eq!(entry["properties"][field]["type"], "string");
            assert!(
                entry["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(field))
            );
        }
        assert_ne!(
            entry["additionalProperties"], false,
            "upstream may add fields"
        );
    }
}

// spec: AGT-007
#[test]
fn type_schemas_are_titled_by_their_key() {
    let schemas = type_schemas();
    let object = schemas.as_object().expect("schemas keyed by type name");
    for (name, schema) in object {
        assert_eq!(schema["title"], json!(name), "{name}");
    }
    let index_event = &schemas["IndexEvent"];
    assert_eq!(
        index_event["oneOf"].as_array().map(Vec::len),
        Some(9),
        "one schema per index event kind"
    );
}

// spec: AGT-007
#[test]
fn hand_written_schemas_reach_the_method_schemas() {
    let schemas = method_schemas();
    let respond = &schemas["interaction.respond"]["params"];
    let variants = respond["$defs"]["InteractionResponse"]["oneOf"]
        .as_array()
        .expect("one schema per response kind");
    assert_eq!(variants.len(), 4);
    assert_eq!(
        variants[2]["properties"]["cancelled"],
        json!({ "const": true }),
        "the constant inline"
    );
    let subscribe = &schemas["subscribe"]["params"];
    assert_eq!(
        subscribe["properties"]["topic"],
        json!({ "$ref": "#/$defs/Topic" })
    );
    assert_eq!(
        subscribe["$defs"]["Topic"]["pattern"],
        json!("^(index|(thread|workspace|views|job|terminal)/[A-Za-z0-9_-]+)$")
    );
}
