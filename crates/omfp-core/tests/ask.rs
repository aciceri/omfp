//! Boundary tests of `ask` answer validation (DMN-054) and of `form` data shapes (DMN-055), following omp's rules in
//! rpc.md "Inbound response".

use omfp_core::wire::{AskAnswer, AskUiRequest};
use omfp_core::{validate_ask_answers, validate_form_data};
use serde_json::json;

fn request() -> AskUiRequest {
    serde_json::from_value(json!({
        "id": "ui_9",
        "questions": [
            { "id": "db", "question": "Which database?", "options": [{ "label": "Postgres" }, { "label": "SQLite" }] },
            {
                "id": "features",
                "question": "Which features?",
                "options": [{ "label": "Auth" }, { "label": "Billing" }, { "label": "Search" }],
                "multi": true,
            },
        ],
    }))
    .expect("valid ask request")
}

fn answer(id: &str, selected: &[&str], custom: Option<&str>) -> AskAnswer {
    AskAnswer {
        id: id.into(),
        selected_options: selected.iter().map(|label| (*label).to_owned()).collect(),
        custom_input: custom.map(str::to_owned),
    }
}

// spec: DMN-054
#[test]
fn accepts_the_documented_answer_shapes() {
    let request = request();
    for answers in [
        vec![
            answer("db", &[], Some("DuckDB")),
            answer("features", &["Auth", "Search"], None),
        ],
        vec![
            answer("db", &["SQLite"], None),
            answer("features", &[], None),
        ],
        vec![
            answer("db", &[], None),
            answer("features", &["Billing"], Some("and more")),
        ],
        vec![
            answer("db", &["Postgres"], Some("   ")),
            answer("features", &["Auth", "Billing", "Search"], None),
        ],
    ] {
        assert_eq!(
            validate_ask_answers(&request, &answers),
            Ok(()),
            "{answers:?}"
        );
    }
}

// spec: DMN-054
#[test]
fn requires_one_answer_per_question() {
    let request = request();
    let one = [answer("db", &["SQLite"], None)];
    assert!(validate_ask_answers(&request, &one).is_err());
    let three = [
        answer("db", &[], None),
        answer("features", &[], None),
        answer("features", &[], None),
    ];
    assert!(validate_ask_answers(&request, &three).is_err());
    assert!(validate_ask_answers(&request, &[]).is_err());
}

// spec: DMN-054
#[test]
fn requires_question_ids_in_request_order() {
    let request = request();
    let swapped = [answer("features", &[], None), answer("db", &[], None)];
    assert!(validate_ask_answers(&request, &swapped).is_err());
    let wrong = [answer("db", &[], None), answer("feature", &[], None)];
    assert!(validate_ask_answers(&request, &wrong).is_err());
}

// spec: DMN-054
#[test]
fn rejects_unknown_or_inexact_labels() {
    let request = request();
    for label in ["MySQL", "sqlite", "SQLite ", ""] {
        let answers = [answer("db", &[label], None), answer("features", &[], None)];
        assert!(
            validate_ask_answers(&request, &answers).is_err(),
            "{label:?}"
        );
    }
}

// spec: DMN-054
#[test]
fn rejects_duplicate_labels() {
    let request = request();
    let answers = [
        answer("db", &[], None),
        answer("features", &["Auth", "Auth"], None),
    ];
    assert!(validate_ask_answers(&request, &answers).is_err());
}

// spec: DMN-054
#[test]
fn single_select_takes_at_most_one_answer() {
    let request = request();
    let two_options = [
        answer("db", &["Postgres", "SQLite"], None),
        answer("features", &[], None),
    ];
    assert!(validate_ask_answers(&request, &two_options).is_err());
    let option_and_custom = [
        answer("db", &["Postgres"], Some("DuckDB")),
        answer("features", &[], None),
    ];
    assert!(validate_ask_answers(&request, &option_and_custom).is_err());
}

// spec: DMN-054
#[test]
fn errors_name_the_offending_question() {
    let request = request();
    let answers = [answer("db", &[], None), answer("features", &["Nope"], None)];
    let error = validate_ask_answers(&request, &answers).expect_err("unknown label");
    assert!(
        error.contains("features") && error.contains("Nope"),
        "{error}"
    );
}

fn data(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

// spec: DMN-055
#[test]
fn form_data_takes_strings_numbers_and_string_arrays() {
    let valid = data(json!({
        "db": "postgres",
        "replicas": 3,
        "ratio": 0.5,
        "features": ["auth", "Elasticsearch"],
        "none": [],
    }));
    assert_eq!(validate_form_data(&valid), Ok(()));
    assert_eq!(validate_form_data(&data(json!({}))), Ok(()));
}

// spec: DMN-055
#[test]
fn form_data_refuses_any_other_value_naming_the_field() {
    for (name, value) in [
        ("flag", json!(true)),
        ("nothing", json!(null)),
        ("nested", json!({ "x": 1 })),
        ("mixed", json!(["a", 1])),
    ] {
        let mut invalid = data(json!({ "ok": "fine" }));
        invalid.insert(name.to_owned(), value);
        let error = validate_form_data(&invalid).expect_err("an invalid value");
        assert!(error.contains(&format!("{name:?}")), "{error}");
    }
}
