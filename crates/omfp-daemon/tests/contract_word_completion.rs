//! Word completion against the pinned omp (L3): the daemon's predictor omp relays `predict_word` and
//! `predict_word_feedback` (DMN-360, DMN-361). omp's `spelling.autocomplete` is `off` here, so omp answers without
//! starting its machine-wide text-prediction daemon, which would outlive the test by its 15-minute idle window; the
//! completions themselves are the fake launcher's (`tests/server.rs`).

mod client;
mod common;

use clap::Parser;
use client::Client;
use common::Harness;
use omfp_daemon::config::{Cli, Config};
use omfp_daemon::{Ports, start_with};
use omfp_testkit::omp_bin;
use serde_json::json;

// spec: DMN-360, DMN-361
#[tokio::test(flavor = "multi_thread")]
async fn the_predictor_omp_relays_omps_word_completion() {
    let harness = Harness::new(Vec::new()).await;
    harness
        .env
        .configure(&[], "spelling:\n  autocomplete: \"off\"\n")
        .unwrap();
    let data = tempfile::tempdir().unwrap();
    let omp = omp_bin();
    let workspaces = data.path().join("workspaces");
    let cli = Cli::try_parse_from([
        "daemon",
        "--bind",
        "127.0.0.1:0",
        "--data-dir",
        data.path().to_str().unwrap(),
        "--workspaces-root",
        workspaces.to_str().unwrap(),
        "--omp-bin",
        &omp,
        "--no-title-generation",
    ])
    .unwrap();
    let config = Config::from_cli(cli, None, None).unwrap();
    let mut ports = Ports::resolve(&config).unwrap();
    ports.omp_env = harness.env.prepare_omfp_profile(data.path()).unwrap();
    let daemon = start_with(config, ports).await.unwrap();
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
