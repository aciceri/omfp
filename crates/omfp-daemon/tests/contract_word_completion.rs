//! Word completion against the pinned omp (L3): the daemon's predictor omp relays `predict_word` and
//! `predict_word_feedback` (DMN-360, DMN-361). omp's `spelling.autocomplete` is `off` here, so omp answers without
//! starting its machine-wide text-prediction daemon, which would outlive the test by its 15-minute idle window; the
//! completions themselves are the fake launcher's (`tests/server.rs`).

mod client;
mod common;

use client::Client;
use common::{Harness, omp_daemon};
use serde_json::json;

// spec: DMN-360, DMN-361
#[tokio::test(flavor = "multi_thread")]
async fn the_predictor_omp_relays_omps_word_completion() {
    let harness = Harness::new(Vec::new()).await;
    harness
        .env
        .configure(&[], "spelling:\n  autocomplete: \"off\"\n")
        .unwrap();
    let (daemon, _data) = omp_daemon(&harness, &["--no-title-generation"]).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;

    let predicted = client
        .ok(
            "prompts.predict_word",
            json!({ "text": "Run the te", "cursor": 10 }),
        )
        .await;
    let feedback = client
        .ok(
            "prompts.predict_word_feedback",
            json!({ "text": "Run the te", "cursor": 10, "suggestion": "sts", "accepted": false }),
        )
        .await;

    assert_eq!(predicted, json!({ "suffix": null }), "omp's setting is off");
    assert_eq!(feedback, json!({}));
    assert!(
        data.path().join("predict").join("omp.log").exists(),
        "the predictor omp ran in its own directory"
    );
    daemon.shutdown().await.unwrap();
}
