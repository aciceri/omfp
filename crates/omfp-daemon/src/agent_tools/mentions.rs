//! What agent-facing texts name, for the checks that every tool and `omfp://` URI a surface serves a text about exists
//! on that surface (threads: host tools and URIs; MCP: tools and resources; the widget builder: URIs only).

use crate::widgets::{self, BUILTIN_URL, CHECK_URL, DOCS_URL, SCHEMA_URL, keymap, shell};

use super::catalog::{self, METHODS_URL};

/// The `omfp_…` tool names `text` mentions, each once, in order; Rust paths (`omfp_core::…`) are no tools.
pub(crate) fn tools(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (start, _) in text.match_indices("omfp_") {
        if text[..start]
            .chars()
            .next_back()
            .is_some_and(|before| before.is_ascii_alphanumeric() || before == '_')
        {
            continue;
        }
        let name: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        let name = name.trim_end_matches('_').to_owned();
        if text[start + name.len()..].starts_with("::") {
            continue;
        }
        if !found.contains(&name) {
            found.push(name);
        }
    }
    found
}

/// The `omfp://` URIs `text` mentions, each once, cut before a placeholder (`<name>`, `{name}`) and without a trailing
/// `/` or punctuation: `omfp://protocol/methods/<name>` is `omfp://protocol/methods`. The bare scheme is no URI.
pub(crate) fn uris(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (start, _) in text.match_indices("omfp://") {
        let uri: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '.' | '_' | '-'))
            .collect();
        let uri = uri.trim_end_matches(['/', '.', ':']).to_owned();
        if uri.len() > "omfp://".len() && !found.contains(&uri) {
            found.push(uri);
        }
    }
    found
}

/// The `omfp://` URIs every reader gets (threads' host URIs and the MCP resources): the method catalogue, the docs and
/// their index, the keymap, the schemas and the built-in widgets.
pub(crate) const STATIC_URIS: [&str; 9] = [
    METHODS_URL,
    shell::INDEX_URL,
    DOCS_URL,
    shell::SHELL_URL,
    shell::KEYMAP_URL,
    SCHEMA_URL,
    keymap::SCHEMA_URL,
    crate::jobs::file::SCHEMA_URL,
    BUILTIN_URL,
];

/// The URIs only a thread reads, about its workspace: the widget check and the windows.
pub(crate) const THREAD_URIS: [&str; 2] = [CHECK_URL, shell::WINDOWS_URL];

/// The static texts every reader can read, by URI: the docs, the index, the keymap, the schemas, the catalogue and
/// every method's schemas.
pub(crate) fn static_texts() -> Vec<(String, String)> {
    let mut texts: Vec<(String, String)> = STATIC_URIS
        .iter()
        .filter(|uri| **uri != BUILTIN_URL)
        .map(|uri| {
            let read = catalog::read_uri(uri)
                .unwrap_or_else(|| widgets::read_uri(uri, None))
                .unwrap_or_else(|error| panic!("{uri}: {error}"));
            ((*uri).to_owned(), read.content)
        })
        .collect();
    for method in catalog::methods() {
        let value = catalog::method(method).unwrap();
        texts.push((format!("{METHODS_URL}/{method}"), value.to_string()));
    }
    texts
}

/// The names `text` (served as `served`) mentions that are neither among `tools` nor `uris`, as `served: name`. A link
/// to a thread (`omfp://thread/<id>`, UI-045) is for the user's window, not a URI anyone reads: never missing.
pub(crate) fn missing(served: &str, text: &str, tools: &[&str], uris: &[&str]) -> Vec<String> {
    let unknown_tools = self::tools(text)
        .into_iter()
        .filter(|name| !tools.contains(&name.as_str()));
    let unknown_uris = self::uris(text)
        .into_iter()
        .filter(|uri| uri != crate::conventions::THREAD_LINK && !uris.contains(&uri.as_str()));
    unknown_tools
        .chain(unknown_uris)
        .map(|name| format!("{served}: {name}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_cut_at_placeholders_and_punctuation() {
        assert_eq!(
            tools(
                "call omfp_ui_mount { id }, then omfp_windows. my_omfp_x, `omfp_core::State` and `omfp_call`"
            ),
            ["omfp_ui_mount", "omfp_windows", "omfp_call"]
        );
        assert_eq!(
            uris(
                "read omfp://protocol/methods/<name>, omfp://docs. Or omfp://widgets/builtin/{name}, \
                 omfp://docs/widgets) of `omfp://`"
            ),
            [
                "omfp://protocol/methods",
                "omfp://docs",
                "omfp://widgets/builtin",
                "omfp://docs/widgets"
            ]
        );
    }

    // spec: AGT-045
    #[test]
    fn a_link_to_a_thread_is_no_uri_a_reader_misses() {
        assert_eq!(
            missing(
                "note",
                "link it as [<title>](omfp://thread/<id>), see omfp://nothing",
                &[],
                &[]
            ),
            ["note: omfp://nothing"]
        );
    }
}
