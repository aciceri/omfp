//! Logging in to a provider against the pinned omp (L3): a fresh install — an isolated omp without any model or
//! credential — where every thread start fails and `auth.status` counts no usable model while a configured omp counts
//! some, `auth.providers` from the daemon's login omp, the start of omp's Anthropic OAuth login up to the URL the
//! window must open, its cancel, and a logout with nothing stored (DMN-219..DMN-223, DMN-235). The OAuth providers
//! cannot finish offline: completing a login is the fake launcher's (`tests/server.rs`).

mod client;
mod common;

use client::Client;
use common::{Harness, omp_daemon};
use omfp_daemon::DaemonHandle;
use serde_json::{Value, json};
use tempfile::TempDir;

/// A daemon without configured models or credentials, with keyless discovery disabled too.
async fn fresh_install(harness: &Harness) -> (DaemonHandle, TempDir) {
    std::fs::remove_file(harness.env.agent_dir().join("models.yml")).unwrap();
    // Scope model selection to one credentialed provider so a newly added keyless provider cannot make the
    // supposedly empty profile usable.
    std::fs::write(
        harness.env.agent_dir().join("config.yml"),
        "enabledModels: [anthropic/claude-sonnet-4-5]\n\
         disabledProviders: [local, web, ollama, llama.cpp, lm-studio, apple]\n",
    )
    .unwrap();
    omp_daemon(harness, &["--no-title-generation"]).await
}

/// The next window request `client` receives, skipping other frames.
async fn window_request(client: &mut Client) -> Value {
    loop {
        let frame = client.frame().await;
        if frame["type"] == "window_request" {
            return frame["request"].clone();
        }
    }
}

// spec: DMN-219, PRT-129, DMN-235
#[tokio::test(flavor = "multi_thread")]
async fn on_a_fresh_install_threads_cannot_start_and_the_login_omp_lists_the_providers() {
    let harness = Harness::new(Vec::new()).await;
    let (daemon, _data) = fresh_install(&harness).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;

    // omp in RPC mode exits at startup because the only allowed model has no usable credential.
    let error = client
        .call(
            "threads.create",
            json!({ "workspace": { "new": {} }, "prompt": "hello" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error["code"], "thread_failed", "{error}");

    let providers = client.ok("auth.providers", json!({})).await;
    let anthropic = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["id"] == "anthropic")
        .unwrap_or_else(|| panic!("no anthropic in {providers}"));
    assert_eq!(anthropic["available"], true);
    assert_eq!(anthropic["authenticated"], false);
    assert!(
        anthropic["name"].as_str().unwrap().contains("Anthropic"),
        "{anthropic}"
    );

    // The same refusal, without starting a thread: the window's cue to propose a login.
    assert_eq!(
        client.ok("auth.status", json!({})).await["models"],
        json!(0)
    );

    let logout = client
        .ok("auth.logout", json!({ "providerId": "anthropic" }))
        .await;
    assert_eq!(
        logout,
        json!({ "providerId": "anthropic", "removed": 0, "remainingSource": null })
    );
    daemon.shutdown().await.unwrap();
    harness.assert_no_stray_network();
}

// spec: DMN-235
#[tokio::test(flavor = "multi_thread")]
async fn a_configured_omp_counts_its_usable_models() {
    let harness = Harness::new(Vec::new()).await;
    let (daemon, _data) = omp_daemon(&harness, &["--no-title-generation"]).await;
    let mut client = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;

    let status = client.ok("auth.status", json!({})).await;

    assert!(status["models"].as_u64().unwrap() > 0, "{status}");
    daemon.shutdown().await.unwrap();
    harness.assert_no_stray_network();
}

// spec: DMN-220, DMN-221, DMN-222, PRT-131, PRT-132
#[tokio::test(flavor = "multi_thread")]
async fn omps_oauth_login_reaches_the_window_as_a_url_to_open_and_can_be_cancelled() {
    let harness = Harness::new(Vec::new()).await;
    let (daemon, _data) = fresh_install(&harness).await;
    let mut window = Client::hello(daemon.addr(), daemon.token().expose(), "test").await;
    window
        .send_text(
            json!({
                "type": "request", "id": "login", "method": "auth.login",
                "params": { "providerId": "anthropic" }
            })
            .to_string(),
        )
        .await;

    let request = window_request(&mut window).await;
    assert_eq!(request["kind"], "login_url", "{request}");
    assert_eq!(request["providerId"], "anthropic");
    let url = request["url"].as_str().unwrap();
    assert!(url.starts_with("https://"), "{url}");
    assert!(url.contains("code_challenge="), "an OAuth PKCE URL: {url}");

    let mut other = Client::hello(daemon.addr(), daemon.token().expose(), "other").await;
    assert_eq!(
        other.ok("auth.login_cancel", json!({})).await,
        json!({ "providerId": "anthropic" })
    );
    let response = loop {
        let frame = window.frame().await;
        if frame["type"] == "response" && frame["id"] == "login" {
            break frame;
        }
    };
    assert_eq!(response["error"]["code"], "conflict", "{response}");
    daemon.shutdown().await.unwrap();
    harness.assert_no_stray_network();
}
