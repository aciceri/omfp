//! Finding files of a workspace by a bare or partial name (DMN-092): `theme.ts` or `src/theme.ts` when the path as
//! given does not exist. The files are those of the workspace's repositories as git lists them (tracked, plus
//! untracked ones that are not ignored) and the folder's own files outside them; files a thread's tool cards name
//! rank first among candidates that match alike.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::{Component, Path};

use omfp_core::ThreadState;
use omfp_protocol::{FileCandidate, FileCandidates, FileMatch};
use serde_json::Value;

use super::files::{Resolved, resolve};
use super::{Discovered, Result, Workspaces};

/// At most this many candidates are returned.
pub const MAX_CANDIDATES: usize = 50;

/// One file of the workspace: its path relative to the folder and, inside a repository, the repository's name and
/// the path relative to it.
struct Listed {
    path: String,
    repo: Option<(String, String)>,
}

/// How `path` matches `query` (both relative, `/`-separated); `inside` is the path in its repository.
fn matches(query: &str, path: &str, inside: Option<&str>) -> Option<FileMatch> {
    if path == query || inside == Some(query) {
        return Some(FileMatch::Exact);
    }
    if query.contains('/')
        && path
            .strip_suffix(query)
            .is_some_and(|before| before.ends_with('/'))
    {
        return Some(FileMatch::Suffix);
    }
    let name = query.rsplit('/').next().unwrap_or(query);
    let base = path.rsplit('/').next().unwrap_or(path);
    (base == name).then_some(FileMatch::Basename)
}

/// `query` is already lowercased and has no whitespace; matching allocates no copy of each file's path.
fn fuzzy_matches(query: &str, path: &str) -> bool {
    let mut path = path.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .all(|wanted| path.any(|letter| letter == wanted))
}

/// `path` relative to `folder`, `/`-separated.
fn relative_text(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// The folder's own non-hidden files outside its repositories; traversal must not cut off later matches.
fn folder_files(folder: &Path, repos: &[Discovered]) -> Vec<String> {
    let skipped: BTreeSet<&Path> = repos.iter().map(|repo| repo.path.as_path()).collect();
    let mut files = Vec::new();
    let mut pending = vec![folder.to_owned()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_name().to_string_lossy().starts_with('.')
                || skipped.contains(path.as_path())
            {
                continue;
            }
            // Links are not followed: a linked folder outside the repositories is not the workspace's to search.
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(path),
                Ok(_) => {
                    if let Ok(relative) = path.strip_prefix(folder) {
                        files.push(relative_text(relative));
                    }
                }
                Err(_) => {}
            }
        }
    }
    files
}

/// The file paths the tool cards of a thread name (UI-116): the path of `read`, `edit` and `write`, and each file of
/// `grep` and `glob` results, as the agent wrote them (relative to the workspace folder or absolute).
#[must_use]
pub fn named_files(state: &ThreadState) -> BTreeSet<String> {
    let mut named = BTreeSet::new();
    for run in &state.tools {
        card_paths(
            &run.tool_name,
            run.args.as_ref(),
            run.result.as_ref().and_then(|result| result.get("details")),
            &mut named,
        );
    }
    for entry in &state.transcript {
        let Ok(message) = serde_json::to_value(&entry.message) else {
            continue;
        };
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                let blocks = message.get("content").and_then(Value::as_array);
                for block in blocks.into_iter().flatten() {
                    if block.get("type").and_then(Value::as_str) == Some("toolCall") {
                        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                        card_paths(name, block.get("arguments"), None, &mut named);
                    }
                }
            }
            Some("toolResult") => {
                let name = message
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                card_paths(name, None, message.get("details"), &mut named);
            }
            _ => {}
        }
    }
    named
}

/// `path:50-80`, `path:raw:5` (omp's selectors) without the selector.
fn without_selector(path: &str) -> &str {
    let mut path = path;
    while let Some((head, tail)) = path.rsplit_once(':') {
        let selector = tail == "raw"
            || (!tail.is_empty()
                && tail
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | ',')));
        if !selector || head.is_empty() {
            break;
        }
        path = head;
    }
    path
}

/// The file a hashline `edit` input targets: its first `[path#TAG]` header.
fn edit_header(input: &str) -> Option<&str> {
    input.lines().find_map(|line| {
        let inner = line.strip_prefix('[')?;
        let (path, rest) = inner.split_once('#')?;
        let tag = rest.get(..4)?;
        (tag.chars().all(|c| c.is_ascii_alphanumeric())
            && rest[4..].starts_with(']')
            && !path.is_empty())
        .then_some(path)
    })
}

/// Adds the paths one tool card names: from its arguments and, for searches, its result's `details`.
fn card_paths(
    name: &str,
    args: Option<&Value>,
    details: Option<&Value>,
    named: &mut BTreeSet<String>,
) {
    let arg = |key: &str| args.and_then(|args| args.get(key)).and_then(Value::as_str);
    let mut add = |path: &str| {
        if !path.is_empty() {
            named.insert(path.to_owned());
        }
    };
    match name {
        "read" => {
            if let Some(path) = arg("path").filter(|path| !path.contains("://")) {
                add(without_selector(path));
            }
        }
        "edit" => {
            if let Some(path) = arg("path").or_else(|| arg("input").and_then(edit_header)) {
                add(path);
            }
        }
        "write" => {
            if let Some(path) = arg("path") {
                add(path);
            }
        }
        "grep" | "glob" => {
            let Some(details) = details else {
                return;
            };
            let matched = details.get("fileMatches").and_then(Value::as_array);
            for file in matched.into_iter().flatten() {
                if let Some(path) = file.get("path").and_then(Value::as_str) {
                    add(path);
                }
            }
            let files = details.get("files").and_then(Value::as_array);
            for file in files.into_iter().flatten() {
                if let Some(path) = file.as_str() {
                    add(path);
                }
            }
        }
        _ => {}
    }
}

/// `mentioned` paths as paths relative to the workspace folder: relative ones as they are, absolute ones inside the
/// folder or inside a repository's target (`<repo>/<rest>`); others dropped.
fn mentioned_in(
    folder: &Path,
    repos: &[Discovered],
    mentioned: &BTreeSet<String>,
) -> BTreeSet<String> {
    let canonical = std::fs::canonicalize(folder).ok();
    mentioned
        .iter()
        .filter_map(|path| {
            let path = Path::new(path);
            if path.is_relative() {
                return Some(relative_text(path));
            }
            [Some(folder), canonical.as_deref()]
                .into_iter()
                .flatten()
                .find_map(|folder| path.strip_prefix(folder).ok())
                .map(relative_text)
                .or_else(|| {
                    repos.iter().filter(|repo| !repo.is_root).find_map(|repo| {
                        path.strip_prefix(&repo.canonical)
                            .ok()
                            .map(|rest| relative_text(&Path::new(&repo.name).join(rest)))
                    })
                })
        })
        .collect()
}

impl Workspaces {
    /// The files of workspace `id` that `query` names (DMN-092), best first: the exact path (relative to the folder
    /// or to a repository), then paths ending with the query, then files with its name; among equal matches, files
    /// in `mentioned` (paths a thread's tool cards name) first, then shorter paths. At most [`MAX_CANDIDATES`].
    /// With `fuzzy`, also match case-insensitive subsequences (whitespace ignored); an empty query lists files.
    /// Strict lookup still resolves paths and names no workspace file for an absolute path outside the folder.
    pub fn find_file(
        &self,
        id: &str,
        query: &str,
        mentioned: &BTreeSet<String>,
        fuzzy: bool,
    ) -> Result<FileCandidates> {
        let ws = self.get(id)?;
        let query = if fuzzy {
            Cow::Borrowed(query.trim())
        } else {
            match resolve(&ws.path, query.trim())? {
                Resolved::Inside(relative) => Cow::Owned(relative_text(&relative)),
                Resolved::Outside(_) => {
                    return Ok(FileCandidates {
                        candidates: Vec::new(),
                        truncated: false,
                    });
                }
            }
        };
        let fuzzy_query = fuzzy.then(|| {
            query
                .chars()
                .filter(|letter| !letter.is_whitespace())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        });
        let repos = self.discovered(id)?;
        let mut listed = Vec::new();
        for repo in &repos {
            for inside in self.git.list_files(&repo.path)? {
                let path = if repo.is_root {
                    inside.clone()
                } else {
                    format!("{}/{inside}", repo.name)
                };
                listed.push(Listed {
                    path,
                    repo: Some((repo.name.clone(), inside)),
                });
            }
        }
        if !repos.iter().any(|repo| repo.is_root) {
            listed.extend(
                folder_files(&ws.path, &repos)
                    .into_iter()
                    .map(|path| Listed { path, repo: None }),
            );
        }
        let mentioned = mentioned_in(&ws.path, &repos, mentioned);
        let mut found: Vec<FileCandidate> = listed
            .into_iter()
            .filter_map(|file| {
                let inside = file.repo.as_ref().map(|(_, inside)| inside.as_str());
                let matched = matches(&query, &file.path, inside).or_else(|| {
                    fuzzy_query
                        .as_deref()
                        .filter(|query| fuzzy_matches(query, &file.path))
                        .map(|_| FileMatch::Fuzzy)
                })?;
                let is_mentioned = mentioned.contains(&file.path)
                    || inside.is_some_and(|inside| mentioned.contains(inside));
                // A tracked file deleted from the working tree is listed by git but cannot be opened.
                std::fs::metadata(ws.path.join(&file.path))
                    .is_ok_and(|meta| meta.is_file())
                    .then(|| FileCandidate {
                        path: file.path,
                        repo: file.repo.map(|(name, _)| name),
                        matched,
                        mentioned: is_mentioned,
                    })
            })
            .collect();
        found.sort_by(|a, b| {
            (
                a.matched,
                !a.mentioned,
                a.path.matches('/').count(),
                &a.path,
            )
                .cmp(&(
                    b.matched,
                    !b.mentioned,
                    b.path.matches('/').count(),
                    &b.path,
                ))
        });
        found.dedup_by(|a, b| a.path == b.path);
        let truncated = found.len() > MAX_CANDIDATES;
        found.truncate(MAX_CANDIDATES);
        Ok(FileCandidates {
            candidates: found,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // spec: DMN-092
    #[test]
    fn a_query_matches_exactly_by_suffix_or_by_name() {
        assert_eq!(
            matches("src/a.ts", "src/a.ts", None),
            Some(FileMatch::Exact)
        );
        assert_eq!(
            matches("src/a.ts", "app/src/a.ts", Some("src/a.ts")),
            Some(FileMatch::Exact)
        );
        assert_eq!(
            matches("src/a.ts", "app/lib/src/a.ts", Some("lib/src/a.ts")),
            Some(FileMatch::Suffix)
        );
        assert_eq!(
            matches("a.ts", "app/src/a.ts", Some("src/a.ts")),
            Some(FileMatch::Basename)
        );
        assert_eq!(
            matches("lib/a.ts", "app/src/a.ts", Some("src/a.ts")),
            Some(FileMatch::Basename)
        );
        // A suffix ends at a component boundary.
        assert_eq!(matches("rc/a.ts", "src/b.ts", None), None);
        assert_eq!(matches("c/a.ts", "src/xa.ts", None), None);
        assert_eq!(matches("a.ts", "src/xa.ts", None), None);
    }

    // spec: DMN-092
    #[test]
    fn selectors_and_edit_headers_give_the_named_path() {
        assert_eq!(without_selector("src/a.rs:10-20"), "src/a.rs");
        assert_eq!(without_selector("src/a.rs:raw:5"), "src/a.rs");
        assert_eq!(without_selector("src/a.rs"), "src/a.rs");
        assert_eq!(edit_header("[src/a.rs#1A2B]\nPUT 1:\n+x"), Some("src/a.rs"));
        assert_eq!(edit_header("PUT 1:\n+x"), None);
    }

    // spec: DMN-092
    #[test]
    fn tool_cards_name_the_paths_of_reads_edits_writes_and_search_results() {
        let mut named = BTreeSet::new();
        card_paths(
            "read",
            Some(&json!({ "path": "a/read.rs:3-9" })),
            None,
            &mut named,
        );
        card_paths(
            "read",
            Some(&json!({ "path": "https://example.com/x" })),
            None,
            &mut named,
        );
        card_paths(
            "edit",
            Some(&json!({ "input": "[b/edit.rs#ZZ99]\n" })),
            None,
            &mut named,
        );
        card_paths(
            "write",
            Some(&json!({ "path": "/abs/write.rs" })),
            None,
            &mut named,
        );
        card_paths(
            "grep",
            None,
            Some(&json!({ "fileMatches": [{ "path": "g/one.rs", "count": 2 }] })),
            &mut named,
        );
        card_paths(
            "glob",
            None,
            Some(&json!({ "files": ["g/two.rs"] })),
            &mut named,
        );
        card_paths(
            "bash",
            Some(&json!({ "command": "cat x.rs" })),
            None,
            &mut named,
        );
        assert_eq!(
            named.into_iter().collect::<Vec<_>>(),
            [
                "/abs/write.rs",
                "a/read.rs",
                "b/edit.rs",
                "g/one.rs",
                "g/two.rs"
            ]
        );
    }
}
