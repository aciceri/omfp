//! Topic names (`index`, `thread/<id>`, `workspace/<id>`, `views/<id>`, `job/<id>`, `terminal/<id>`).

use omfp_protocol::Topic;
use serde_json::json;

use crate::samples::{THREAD_ID, WORKSPACE_ID};

// spec: TRM-100
#[test]
fn topics_round_trip_through_their_names() {
    let cases = [
        (Topic::Index, "index".to_owned()),
        (
            Topic::Thread(THREAD_ID.to_owned()),
            format!("thread/{THREAD_ID}"),
        ),
        (
            Topic::Workspace(WORKSPACE_ID.to_owned()),
            format!("workspace/{WORKSPACE_ID}"),
        ),
        (
            Topic::Views(WORKSPACE_ID.to_owned()),
            format!("views/{WORKSPACE_ID}"),
        ),
        (Topic::Job("01JOB".to_owned()), "job/01JOB".to_owned()),
        (
            Topic::Terminal("01TERMINAL".to_owned()),
            "terminal/01TERMINAL".to_owned(),
        ),
        // Ids are any non-empty `[A-Za-z0-9_-]` string.
        (Topic::Job("a-b_C9".to_owned()), "job/a-b_C9".to_owned()),
    ];
    for (topic, name) in cases {
        assert_eq!(topic.to_string(), name);
        assert_eq!(name.parse::<Topic>().unwrap(), topic);
        assert_eq!(serde_json::to_value(&topic).unwrap(), json!(name));
        assert_eq!(serde_json::from_value::<Topic>(json!(name)).unwrap(), topic);
    }
}

// spec: PRT-135
#[test]
fn malformed_topics_are_rejected() {
    for name in [
        "",
        "index/",
        "index/1",
        "thread",
        "thread/",
        "thread/a/b",
        "thread/a b",
        "Thread/01J",
        "threads/01J",
        "repo/01J",
        "/01J",
        " index",
    ] {
        let error = name.parse::<Topic>().unwrap_err();
        assert!(error.to_string().contains("topic"), "{name:?}: {error}");
        assert!(
            serde_json::from_value::<Topic>(json!(name)).is_err(),
            "{name:?}"
        );
    }
}
