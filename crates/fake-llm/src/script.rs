use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An ordered list of replies the fake model gives, one per chat request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    model: String,
    steps: Vec<Step>,
}

impl Script {
    #[must_use]
    pub fn new(model: impl Into<String>, steps: Vec<Step>) -> Self {
        Self {
            model: model.into(),
            steps,
        }
    }

    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    #[must_use]
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub(crate) fn into_parts(self) -> (String, Vec<Step>) {
        (self.model, self.steps)
    }
}

/// One expected request and the reply to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Matcher>,
    pub reply: Reply,
    /// Hold the reply until the test calls `FakeLlm::release_gate` for this step, keeping omp busy deterministically.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gated: bool,
    /// May answer a matching request that arrives while other steps are still ahead of it (concurrent agents: a
    /// background subagent next to its parent). Needs `when`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub any_order: bool,
}

impl Step {
    /// Replies to the next request, whatever it contains.
    #[must_use]
    pub fn reply(reply: Reply) -> Self {
        Self {
            when: None,
            reply,
            gated: false,
            any_order: false,
        }
    }

    /// Replies only if the request satisfies `matcher`; otherwise the request fails the run.
    #[must_use]
    pub fn when(matcher: Matcher, reply: Reply) -> Self {
        Self {
            when: Some(matcher),
            reply,
            gated: false,
            any_order: false,
        }
    }

    /// Holds this step's reply until released (see `FakeLlm::wait_for_gate` / `FakeLlm::release_gate`).
    #[must_use]
    pub fn gated(mut self) -> Self {
        self.gated = true;
        self
    }

    /// Lets this `when` step answer a matching request before the steps ahead of it: a request the front step does not
    /// match goes to the first matching `any_order` step instead of failing.
    #[must_use]
    pub fn any_order(mut self) -> Self {
        self.any_order = true;
        self
    }
}

/// Condition a request must meet for its step to answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Matcher {
    /// The text of the last `user` message contains this substring.
    LastUserContains(String),
}

impl Matcher {
    pub(crate) fn matches(&self, request: &Value) -> bool {
        match self {
            Self::LastUserContains(needle) => {
                last_user_text(request).is_some_and(|text| text.contains(needle.as_str()))
            }
        }
    }
}

/// Text of the last `user` message, whether `content` is a string or a list of parts.
pub(crate) fn last_user_text(request: &Value) -> Option<String> {
    let message = request["messages"]
        .as_array()?
        .iter()
        .rev()
        .find(|message| message["role"] == "user")?;
    match &message["content"] {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect(),
        ),
        _ => None,
    }
}

/// What the assistant answers: optional text, then optional tool calls.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

impl Reply {
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            tool_calls: Vec::new(),
        }
    }

    #[must_use]
    pub fn tool_calls(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            text: None,
            tool_calls,
        }
    }

    pub(crate) fn finish_reason(&self) -> &'static str {
        if self.tool_calls.is_empty() {
            "stop"
        } else {
            "tool_calls"
        }
    }
}

/// A function call the assistant makes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

impl ToolCall {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}
