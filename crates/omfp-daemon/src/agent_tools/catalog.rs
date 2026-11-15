//! The method catalogue (AGT-042): what `omfp_call` accepts, read from the protocol's method table (PRT-092) and its
//! JSON Schemas (AGT-007), served as `omfp://` host URIs and reusable by other adapters (MCP resources).

use std::fmt::Write as _;

use omfp_protocol::{METHODS, method_description, method_schemas};
use omp_rpc::{HostUriRead, HostUriResultContentType};
use serde_json::{Value, json};

use super::{CALL, not_for_agents};

/// Host URI listing the methods; `<METHODS_URL>/<name>` reads one method's schemas.
pub const METHODS_URL: &str = "omfp://protocol/methods";

/// The methods an agent may call with `omfp_call`, in method-table order: all but the connection-only ones.
pub fn methods() -> impl Iterator<Item = &'static str> {
    METHODS
        .iter()
        .copied()
        .filter(|method| not_for_agents(method).is_none())
}

/// `omfp://protocol/methods`: one line per method with its description (Markdown).
#[must_use]
pub fn list() -> String {
    let mut text = format!(
        "# omfp protocol methods\n\nCall one with {CALL} {{ method, params }}; read {METHODS_URL}/<name> for its \
         params and result JSON Schemas.\n\n"
    );
    for method in methods() {
        let description = method_description(method).unwrap_or_default();
        let _ = writeln!(text, "- `{method}` — {description}");
    }
    text
}

/// `omfp://protocol/methods/<name>`: `{ method, description, params, result }`, or why there is no such method.
pub fn method(name: &str) -> Result<Value, String> {
    if let Some(why) = not_for_agents(name) {
        return Err(why);
    }
    let schemas = method_schemas();
    match (method_description(name), schemas.get(name)) {
        (Some(description), Some(schema)) => Ok(json!({
            "method": name,
            "description": description,
            "params": schema["params"],
            "result": schema["result"],
        })),
        _ => Err(format!(
            "unknown omfp protocol method `{name}`; {METHODS_URL} lists the methods {CALL} accepts"
        )),
    }
}

/// What a catalogue URL reads (AGT-042); `None` when `url` is not one.
#[must_use]
pub fn read_uri(url: &str) -> Option<Result<HostUriRead, String>> {
    let url = url.trim_end_matches('/');
    if url == METHODS_URL {
        return Some(Ok(HostUriRead {
            content: list(),
            content_type: Some(HostUriResultContentType::TextMarkdown),
            immutable: Some(true),
            ..HostUriRead::default()
        }));
    }
    let name = url.strip_prefix(METHODS_URL)?.strip_prefix('/')?;
    Some(method(name).and_then(|value| {
        Ok(HostUriRead {
            content: serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?,
            content_type: Some(HostUriResultContentType::ApplicationJson),
            immutable: Some(true),
            ..HostUriRead::default()
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::super::CONNECTION_ONLY;
    use super::*;

    // spec: AGT-042, PRT-092
    #[test]
    fn the_list_names_every_method_an_agent_may_call_with_its_description() {
        let read = read_uri("omfp://protocol/methods/").unwrap().unwrap();
        assert_eq!(
            read.content_type,
            Some(HostUriResultContentType::TextMarkdown)
        );
        assert_eq!(read.immutable, Some(true));
        let text = read.content;
        assert!(text.starts_with("# omfp protocol methods\n"), "{text}");
        assert!(text.contains("omfp://protocol/methods/<name>"), "{text}");
        let lines: Vec<&str> = text
            .lines()
            .filter(|line| line.starts_with("- `"))
            .collect();
        assert_eq!(lines.len(), METHODS.len() - CONNECTION_ONLY.len());
        assert!(
            lines.contains(
                &"- `threads.list` — Thread summaries as on the `index` topic, without archived threads unless \
                  asked (PRT-090)."
            ),
            "{text}"
        );
        for method in CONNECTION_ONLY {
            assert!(!text.contains(&format!("`{method}`")), "{method}: {text}");
        }
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("- `terminals.start` — ")),
            "{text}"
        );
        // Table order: the first callable method comes first.
        assert!(lines[0].starts_with("- `repos.suggest` — "), "{text}");
    }

    // spec: AGT-042, AGT-007
    #[test]
    fn a_method_url_gives_its_description_and_schemas() {
        let read = read_uri("omfp://protocol/methods/threads.create")
            .unwrap()
            .unwrap();
        assert_eq!(
            read.content_type,
            Some(HostUriResultContentType::ApplicationJson)
        );
        assert_eq!(read.immutable, Some(true));
        let value: Value = serde_json::from_str(&read.content).unwrap();
        let schemas = method_schemas();
        assert_eq!(value["method"], "threads.create");
        assert_eq!(
            value["description"],
            "Creates a thread in an existing or a new workspace."
        );
        assert_eq!(value["params"], schemas["threads.create"]["params"]);
        assert_eq!(value["result"], schemas["threads.create"]["result"]);
    }

    // spec: AGT-042
    #[test]
    fn unknown_and_connection_only_methods_name_the_list() {
        for (url, reason) in [
            (
                "omfp://protocol/methods/threads.nothing",
                "unknown omfp protocol method `threads.nothing`",
            ),
            (
                "omfp://protocol/methods/subscribe",
                "needs a window's connection",
            ),
            (
                "omfp://protocol/methods/terminal.open",
                "unknown omfp protocol method `terminal.open`",
            ),
        ] {
            let error = read_uri(url).unwrap().unwrap_err();
            assert!(error.contains(reason), "{url}: {error}");
            assert!(error.contains(METHODS_URL), "{url}: {error}");
        }
        assert!(read_uri("omfp://docs/widgets").is_none());
        assert!(read_uri("omfp://protocol/methodsx").is_none());
    }
}
