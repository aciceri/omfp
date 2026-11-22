//! Rules for decoding client frames (`decode_request`).

use omfp_protocol::{
    ApprovalMode, Call, Empty, ErrorCode, ProtocolError, Request, SetViewsParams, StopThreadParams,
    ViewsFile, decode_request,
};
use serde_json::json;

use crate::samples::THREAD_ID;

fn decode(value: &serde_json::Value) -> Result<Request, ProtocolError> {
    decode_request(&value.to_string())
}

// spec: DMN-210
#[test]
fn an_approval_mode_passes_to_omp_as_its_wire_value() {
    for (mode, value) in [
        (ApprovalMode::AlwaysAsk, "always-ask"),
        (ApprovalMode::AlwaysAskCached, "always-ask-cached"),
        (ApprovalMode::Write, "write"),
        (ApprovalMode::Yolo, "yolo"),
    ] {
        assert_eq!(mode.as_str(), value);
        assert_eq!(serde_json::to_value(mode).unwrap(), json!(value));
    }
}

// spec: PRT-010
#[test]
fn a_request_keeps_its_client_chosen_id() {
    let request = decode(&json!({
        "type": "request", "id": "c42", "method": "thread.stop", "params": { "threadId": THREAD_ID }
    }))
    .unwrap();
    assert_eq!(
        request,
        Request::new(
            "c42",
            Call::ThreadStop(StopThreadParams {
                thread_id: THREAD_ID.to_owned(),
                cascade: false,
            })
        )
    );
}

// spec: PRT-010, PRT-011
#[test]
fn a_request_without_id_is_rejected_without_id() {
    for id in [None, Some(json!(42)), Some(json!(null))] {
        let mut frame = json!({ "type": "request", "method": "theme.get", "params": {} });
        if let Some(id) = id {
            frame["id"] = id;
        }
        let error = decode(&frame).unwrap_err();
        assert_eq!(error.code(), ErrorCode::BadRequest);
        assert_eq!(error.id, None);
    }
}

// spec: PRT-012, PRT-011
#[test]
fn an_unknown_method_is_a_bad_request_for_that_id() {
    let error = decode(
        &json!({ "type": "request", "id": "c9", "method": "thread.teleport", "params": {} }),
    )
    .unwrap_err();
    assert_eq!(error.code(), ErrorCode::BadRequest);
    assert_eq!(error.id.as_deref(), Some("c9"));
    let body = error.to_error_body();
    assert_eq!(body.code, ErrorCode::BadRequest);
    assert!(body.message.contains("thread.teleport"), "{}", body.message);
}

// spec: PRT-012
#[test]
fn unknown_params_fields_are_ignored() {
    let request = decode(&json!({
        "type": "request",
        "id": "c3",
        "method": "thread.stop",
        "params": { "threadId": THREAD_ID, "graceful": true, "reason": { "nested": [1, 2] } }
    }))
    .unwrap();
    assert_eq!(
        request.call,
        Call::ThreadStop(StopThreadParams {
            thread_id: THREAD_ID.to_owned(),
            cascade: false,
        })
    );
}

// spec: PRT-012
#[test]
fn unknown_envelope_fields_are_ignored() {
    let request = decode(&json!({
        "type": "request", "id": "c4", "method": "theme.get", "params": {}, "trace": "abc"
    }))
    .unwrap();
    assert_eq!(request, Request::new("c4", Call::ThemeGet(Empty {})));
}

#[test]
fn absent_or_null_params_mean_no_params() {
    for frame in [
        json!({ "type": "request", "id": "c5", "method": "theme.get" }),
        json!({ "type": "request", "id": "c5", "method": "theme.get", "params": null }),
    ] {
        assert_eq!(
            decode(&frame).unwrap(),
            Request::new("c5", Call::ThemeGet(Empty {}))
        );
    }
}

// spec: PRT-011
#[test]
fn invalid_params_are_a_bad_request_for_that_id() {
    for params in [json!({}), json!({ "threadId": 7 }), json!([THREAD_ID])] {
        let error = decode(
            &json!({ "type": "request", "id": "c6", "method": "thread.stop", "params": params }),
        )
        .unwrap_err();
        assert_eq!(error.code(), ErrorCode::BadRequest);
        assert_eq!(error.id.as_deref(), Some("c6"));
    }
}

// spec: PRT-011
#[test]
fn malformed_frames_are_bad_requests() {
    let cases = [
        "not json".to_owned(),
        json!(["request"]).to_string(),
        json!({ "id": "c1", "method": "theme.get" }).to_string(),
        json!({ "type": "response", "id": "c1", "method": "theme.get" }).to_string(),
        json!({ "type": "request", "id": "c1" }).to_string(),
        json!({ "type": "request", "id": "c1", "method": 3 }).to_string(),
    ];
    for text in cases {
        let error = decode_request(&text).unwrap_err();
        assert_eq!(error.code(), ErrorCode::BadRequest, "{text}");
        assert_ne!(error.message, "", "{text}");
        assert_eq!(error.to_string(), error.message, "{text}");
        assert_eq!(error.to_error_body().message, error.message, "{text}");
    }
}

#[test]
fn a_request_deserializes_like_decode_request() {
    let frame = json!({ "type": "request", "id": "c8", "method": "thread.restart", "params": { "threadId": THREAD_ID } });
    let request: Request = serde_json::from_value(frame.clone()).unwrap();
    assert_eq!(request, decode(&frame).unwrap());
    assert_eq!(serde_json::to_value(&request).unwrap(), frame);
}

// spec: DMN-364
#[test]
fn views_set_tells_an_expected_absent_file_from_no_expectation() {
    let params = |expected: Option<serde_json::Value>| {
        let mut params = json!({ "workspaceId": "w1", "manifest": { "views": {} } });
        if let Some(expected) = expected {
            params["expected"] = expected;
        }
        params
    };
    for (sent, expected) in [
        (None, None),
        (Some(json!(null)), Some(ViewsFile::Missing)),
        (
            Some(json!({ "views": {} })),
            Some(ViewsFile::Manifest(json!({ "views": {} }))),
        ),
    ] {
        let decoded: SetViewsParams = serde_json::from_value(params(sent.clone())).unwrap();
        assert_eq!(decoded.expected, expected);
        assert_eq!(serde_json::to_value(&decoded).unwrap(), params(sent));
    }
}
