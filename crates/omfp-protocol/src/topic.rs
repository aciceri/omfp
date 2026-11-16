//! Subscription topics.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A stream of events a client can subscribe to; on the wire, its name (`index`, `thread/<id>`, …).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Topic {
    /// Repositories, workspaces and thread summaries.
    Index,
    /// One thread: `omfp_core::ThreadEvent`s.
    Thread(String),
    /// Git status of one workspace.
    Workspace(String),
    /// Views manifest, widget folders and widget state of one workspace (PRT-040).
    Views(String),
    /// Output of one job (PRT-041).
    Job(String),
    /// Interactive PTY output of one terminal session (TRM-100).
    Terminal(String),
    /// The declared jobs of one workspace and their runs (PRT-200).
    Jobs(String),
}

/// A topic name that is not `index` or `<thread|workspace|views|job|jobs|terminal>/<id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TopicParseError {
    pub name: String,
}

impl fmt::Display for TopicParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid topic {:?}: expected `index` or `<thread|workspace|views|job|jobs|terminal>/<id>`",
            self.name
        )
    }
}

impl std::error::Error for TopicParseError {}

/// Ids are ULIDs today; any non-empty `[A-Za-z0-9_-]` string is accepted.
fn is_valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

impl FromStr for Topic {
    type Err = TopicParseError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        if name == "index" {
            return Ok(Self::Index);
        }
        let topic = match name.split_once('/') {
            Some((kind, id)) if is_valid_id(id) => match kind {
                "thread" => Some(Self::Thread(id.to_owned())),
                "workspace" => Some(Self::Workspace(id.to_owned())),
                "views" => Some(Self::Views(id.to_owned())),
                "job" => Some(Self::Job(id.to_owned())),
                "terminal" => Some(Self::Terminal(id.to_owned())),
                "jobs" => Some(Self::Jobs(id.to_owned())),
                _ => None,
            },
            _ => None,
        };
        topic.ok_or_else(|| TopicParseError {
            name: name.to_owned(),
        })
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Index => f.write_str("index"),
            Self::Thread(id) => write!(f, "thread/{id}"),
            Self::Workspace(id) => write!(f, "workspace/{id}"),
            Self::Views(id) => write!(f, "views/{id}"),
            Self::Job(id) => write!(f, "job/{id}"),
            Self::Terminal(id) => write!(f, "terminal/{id}"),
            Self::Jobs(id) => write!(f, "jobs/{id}"),
        }
    }
}

impl Serialize for Topic {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Topic {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = Cow::<str>::deserialize(deserializer)?;
        name.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for Topic {
    fn schema_name() -> Cow<'static, str> {
        "Topic".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "`index`, `thread/<id>`, `workspace/<id>`, `views/<id>`, `job/<id>`, `jobs/<id>` or `terminal/<id>`.",
            "type": "string",
            "pattern": "^(index|(thread|workspace|views|job|jobs|terminal)/[A-Za-z0-9_-]+)$"
        })
    }
}
