//! The docs of the window itself for agents (DMN-198, DMN-199): the index of the
//! `omfp://` docs, the shell reference `omfp://docs/shell` and `omfp://keymap`, the default bindings and commands of
//! the app (`keymap-defaults.json`, generated from the UI) with what this daemon and workspace add.

use std::fmt::Write as _;

use omfp_protocol::{KeymapEntry, KeymapMode, WorkspaceBinding};
use serde::Deserialize;

use super::{BUILTIN_URL, DOCS_URL, SCHEMA_URL, keymap};
use crate::wire_name;

/// Host URI of the docs index (DMN-198).
pub const INDEX_URL: &str = "omfp://docs";
/// Host URI of [`SHELL`] (DMN-198).
pub const SHELL_URL: &str = "omfp://docs/shell";
/// Host URI of the rendered keymap (DMN-199).
pub const KEYMAP_URL: &str = "omfp://keymap";
/// Host URI describing the open windows showing the thread's workspace (AGT-081), as `omfp_windows` does.
pub const WINDOWS_URL: &str = "omfp://windows";
/// The window's reference: left column, mode line and view tabs, `:` commands, default keys (DMN-198); logging in and
/// language servers (AGT-099).
pub const SHELL: &str = include_str!("shell.md");
/// The app's default bindings and commands, generated from the UI (DMN-199).
pub const KEYMAP_DEFAULTS: &str = include_str!("keymap-defaults.json");

/// `keymap-defaults.json`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Defaults {
    #[serde(default)]
    pub bindings: Vec<DefaultBinding>,
    #[serde(default)]
    pub commands: Vec<DefaultCommand>,
}

/// One default binding of the app.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DefaultBinding {
    pub mode: KeymapMode,
    pub keys: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// One command of the app.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DefaultCommand {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// The widget scope the command applies in while focus is inside it (`transcript`, `threads`); `None` when it
    /// applies everywhere.
    #[serde(default)]
    pub scope: Option<String>,
}

/// [`KEYMAP_DEFAULTS`] parsed.
pub fn defaults() -> Result<Defaults, serde_json::Error> {
    serde_json::from_str(KEYMAP_DEFAULTS)
}

/// What a window adds to the defaults that the daemon knows (DMN-199).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Additions {
    /// The `[keymap]` entries of the daemon's config file.
    pub daemon: Vec<KeymapEntry>,
    /// The workspace's `.omfp/keymap.json` when read for a thread's workspace: its bindings (none without a file), or
    /// why it is refused; `None` when read without a workspace.
    pub workspace: Option<Result<Vec<WorkspaceBinding>, String>>,
}

/// The keymap modes in the order the docs list them: `normal` is the layer Threads and View mode share, where a
/// `threads` or `view` binding of the same keys wins.
const MODES: [KeymapMode; 5] = [
    KeymapMode::Normal,
    KeymapMode::Threads,
    KeymapMode::View,
    KeymapMode::Insert,
    KeymapMode::Cmdline,
];

/// `` `keys` → `command` `` with its arguments; an empty command removes the binding of `keys`.
fn binding_line(keys: &str, command: &str, args: &[String]) -> String {
    if command.is_empty() {
        return format!("`{keys}` → (unbound: removes the default binding)");
    }
    let mut line = format!("`{keys}` → `{command}`");
    if !args.is_empty() {
        let args: Vec<String> = args.iter().map(|arg| format!("`{arg}`")).collect();
        let _ = write!(line, " ({})", args.join(", "));
    }
    line
}

/// `omfp://docs` (DMN-198): one line per docs resource.
#[must_use]
pub fn index() -> String {
    format!(
        "# omfp docs\n\
         \n\
         Read-only `omfp://` resources about the omfp window, for agents:\n\
         \n\
         - {DOCS_URL} — widgets, views (`.omfp/views.json`), init files, the `omfp/sdk` SDK and the \
         `omfp/kit` kit.\n\
         - {SHELL_URL} — the window: left column, mode line and view tabs, `:` commands, default keys; logging in \
         and how omp finds language servers.\n\
         - {KEYMAP_URL} — the default key bindings and commands; a thread's agent also sees what this daemon's \
         config and its workspace's `.omfp/keymap.json` add.\n\
         - {SCHEMA_URL} — the JSON Schema of `.omfp/views.json`.\n\
         - {} — the JSON Schema of `.omfp/keymap.json`.\n\
         - {} — the JSON Schema of `.omfp/jobs.json`, the commands and prompts this workspace runs by hand, on a \
         schedule or as services.\n\
         - {BUILTIN_URL} — the built-in widgets; {BUILTIN_URL}/<name>[/<path>] their sources.\n\
         - {} — the protocol methods `omfp_call` accepts; {}/<name> gives one's params and result schemas.\n\
         \n\
         A thread's read of any of these takes the read tool's line selectors after the URL (`:N`, `:N-M`, `:N+K`, \
         `:-N`, comma lists, `:raw`, `:raw:N-M`); a long answer comes in pages and ends with the selector that \
         reads on, in the document's own line numbers.\n\
         \n\
         Check a widget or an init file (`init`, `user:init`) with `omfp_widget_check`, \
         and what the open windows show with `omfp_windows` ({DOCS_URL}, \"Checking your work in the window\").\n",
        keymap::SCHEMA_URL,
        crate::jobs::file::SCHEMA_URL,
        crate::agent_tools::catalog::METHODS_URL,
        crate::agent_tools::catalog::METHODS_URL,
    )
}

/// `omfp://keymap` (DMN-199): the default bindings by mode, the commands, then the window-specific additions.
#[must_use]
pub fn render_keymap(additions: &Additions) -> String {
    render(&defaults(), additions)
}

/// One command of the defaults: id, title, `:` aliases, description and scope.
fn command_line(text: &mut String, command: &DefaultCommand) {
    let _ = write!(text, "- `{}` — {}", command.id, command.title);
    if !command.aliases.is_empty() {
        let aliases: Vec<String> = command
            .aliases
            .iter()
            .map(|alias| format!("`:{alias}`"))
            .collect();
        let _ = write!(text, " ({})", aliases.join(", "));
    }
    text.push('.');
    if let Some(description) = &command.description {
        let _ = write!(text, " {description}");
    }
    if let Some(scope) = &command.scope {
        let _ = write!(text, " Scope `{scope}`: applies while focus is inside it.");
    }
    text.push('\n');
}

/// [`render_keymap`] of `defaults`.
fn render(defaults: &Result<Defaults, serde_json::Error>, additions: &Additions) -> String {
    let mut text = format!(
        "# omfp keys and commands\n\
         \n\
         The key bindings and `:` commands every omfp window has by default (see {SHELL_URL} for the window). A \
         binding runs a command by id, with its arguments; `:` runs a command by an alias. `threads` mode: nothing \
         is focused or the focus is in the left column; `view` mode: the focus is in the view, outside a text \
         field; a `normal` binding applies in both, unless a `threads` or `view` binding has the same keys.\n"
    );
    match defaults {
        Ok(defaults) => {
            text.push_str("\n## Default bindings\n");
            for mode in MODES {
                let bindings: Vec<&DefaultBinding> = defaults
                    .bindings
                    .iter()
                    .filter(|binding| binding.mode == mode)
                    .collect();
                if bindings.is_empty() {
                    continue;
                }
                let _ = writeln!(text, "\n### {} mode\n", wire_name(mode));
                for binding in bindings {
                    let _ = writeln!(
                        text,
                        "- {}",
                        binding_line(&binding.keys, &binding.command, &binding.args)
                    );
                }
            }
            text.push_str(
                "\n## Commands\n\nThe commands a window showing the default view with a thread registers; some come \
                 from built-in widgets and exist only while those are shown.\n\n",
            );
            for command in &defaults.commands {
                command_line(&mut text, command);
            }
        }
        Err(error) => {
            let _ = writeln!(text, "\nThe default bindings cannot be read: {error}");
        }
    }
    text.push_str("\n## Window-specific additions\n\n");
    if additions.daemon.is_empty() {
        text.push_str("- This daemon's config file has no `[keymap]` entries.\n");
    } else {
        text.push_str(
            "- This daemon's config file `[keymap]` (settings.get), over the defaults in every window:\n",
        );
        for entry in &additions.daemon {
            let _ = writeln!(
                text,
                "  - {} mode: {}",
                wire_name(entry.mode),
                binding_line(&entry.keys, &entry.command, &[])
            );
        }
    }
    match &additions.workspace {
        None => {}
        Some(Ok(bindings)) if bindings.is_empty() => text.push_str(
            "- This workspace's `.omfp/keymap.json` has no bindings (schema omfp://schemas/keymap.json).\n",
        ),
        Some(Ok(bindings)) => {
            text.push_str(
                "- This workspace's `.omfp/keymap.json`, in windows showing this workspace (schema \
                 omfp://schemas/keymap.json):\n",
            );
            for binding in bindings {
                let _ = writeln!(
                    text,
                    "  - {} mode: {}",
                    wire_name(binding.mode),
                    binding_line(&binding.keys, &binding.command, &binding.args)
                );
            }
        }
        Some(Err(error)) => {
            let _ = writeln!(
                text,
                "- This workspace's `.omfp/keymap.json` is invalid ({error}): windows keep its last valid bindings."
            );
        }
    }
    text.push_str(
        "- Init files (`command`, `bind`) and the workspace's widgets (`useCommand`) add commands and bindings per \
         window: see them in a window with `:describe-command`, `:describe-key` and `:help`.\n",
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-199
    #[test]
    fn the_embedded_defaults_parse_and_every_binding_and_command_is_rendered() {
        let defaults = defaults().expect("keymap-defaults.json parses");
        assert!(!defaults.bindings.is_empty(), "bindings");
        assert!(!defaults.commands.is_empty(), "commands");
        let text = render_keymap(&Additions::default());
        for heading in [
            "## Default bindings",
            "## Commands",
            "## Window-specific additions",
        ] {
            assert!(text.contains(heading), "{heading:?} missing: {text}");
        }
        for binding in &defaults.bindings {
            let line = binding_line(&binding.keys, &binding.command, &binding.args);
            assert!(text.contains(&line), "{line:?} missing: {text}");
        }
        for command in &defaults.commands {
            assert!(
                text.contains(&format!("- `{}` — {}", command.id, command.title)),
                "{} missing: {text}",
                command.id
            );
        }
        assert!(
            text.contains("no `[keymap]` entries"),
            "no daemon entries: {text}"
        );
        assert!(
            !text.contains(".omfp/keymap.json`,") && !text.contains("is invalid"),
            "no workspace part without a workspace: {text}"
        );
        assert!(
            text.contains("`:describe-command`, `:describe-key` and `:help`"),
            "{text}"
        );
    }

    // spec: DMN-199
    #[test]
    fn the_keymap_renders_modes_aliases_conditions_and_the_window_specific_additions() {
        let defaults: Defaults = serde_json::from_str(
            r#"{
              "bindings": [
                { "mode": "normal", "keys": "g v", "command": "view.pick", "args": ["ops"] },
                { "mode": "threads", "keys": "j", "command": "thread.next" },
                { "mode": "view", "keys": "j", "command": "nav.next" },
                { "mode": "cmdline", "keys": "Ctrl-p", "command": "history.prev" }
              ],
              "commands": [
                { "id": "thread.new", "title": "New thread", "aliases": ["new", "n"], "description": "Starts one." },
                { "id": "transcript.copy", "title": "Copy", "aliases": [], "scope": "transcript", "unknown": 1 }
              ]
            }"#,
        )
        .unwrap();
        assert_eq!(defaults.commands[1].scope.as_deref(), Some("transcript"));
        let text = render(&Ok(defaults), &Additions::default());
        for line in [
            "### normal mode\n\n- `g v` → `view.pick` (`ops`)\n",
            "### threads mode\n\n- `j` → `thread.next`\n",
            "### view mode\n\n- `j` → `nav.next`\n",
            "### cmdline mode\n\n- `Ctrl-p` → `history.prev`\n",
            "a `normal` binding applies in both",
            "- `thread.new` — New thread (`:new`, `:n`). Starts one.\n",
            "- `transcript.copy` — Copy. Scope `transcript`: applies while focus is inside it.\n",
            "exist only while those are shown",
        ] {
            assert!(text.contains(line), "{line:?} missing: {text}");
        }
        assert!(!text.contains("### insert mode"), "no empty mode: {text}");
        assert_eq!(
            binding_line("g v", "view.pick", &["ops".to_owned()]),
            "`g v` → `view.pick` (`ops`)"
        );
        assert_eq!(
            binding_line("g g", "", &[]),
            "`g g` → (unbound: removes the default binding)"
        );

        let additions = Additions {
            daemon: vec![KeymapEntry {
                mode: KeymapMode::Insert,
                keys: "Ctrl-j".to_owned(),
                command: "nav.next".to_owned(),
            }],
            workspace: Some(Ok(vec![WorkspaceBinding {
                mode: KeymapMode::Normal,
                keys: "Space t".to_owned(),
                command: "tests.run".to_owned(),
                args: vec!["unit".to_owned()],
            }])),
        };
        let text = render_keymap(&additions);
        assert!(
            text.contains("  - insert mode: `Ctrl-j` → `nav.next`\n"),
            "{text}"
        );
        assert!(
            text.contains(
                "This workspace's `.omfp/keymap.json`, in windows showing this workspace"
            ),
            "{text}"
        );
        assert!(
            text.contains("  - normal mode: `Space t` → `tests.run` (`unit`)\n"),
            "{text}"
        );
        let invalid = render_keymap(&Additions {
            daemon: Vec::new(),
            workspace: Some(Err("bindings: expected an array of bindings".to_owned())),
        });
        assert!(
            invalid.contains(
                "`.omfp/keymap.json` is invalid (bindings: expected an array of bindings): windows keep its last \
                 valid bindings."
            ),
            "{invalid}"
        );
        let empty = render_keymap(&Additions {
            daemon: Vec::new(),
            workspace: Some(Ok(Vec::new())),
        });
        assert!(
            empty.contains("`.omfp/keymap.json` has no bindings"),
            "{empty}"
        );
    }

    // spec: DMN-198
    #[test]
    fn the_docs_index_names_every_docs_resource_once() {
        let text = index();
        for url in [
            DOCS_URL,
            SHELL_URL,
            KEYMAP_URL,
            SCHEMA_URL,
            keymap::SCHEMA_URL,
            crate::jobs::file::SCHEMA_URL,
            BUILTIN_URL,
            crate::agent_tools::catalog::METHODS_URL,
        ] {
            assert_eq!(
                text.lines()
                    .filter(|line| line.starts_with(&format!("- {url} — ")))
                    .count(),
                1,
                "{url} has no line or several: {text}"
            );
        }
        assert!(text.contains("`user:init`"), "{text}");
    }

    // spec: AGT-099, DMN-198
    #[test]
    fn the_shell_reference_says_how_omp_finds_language_servers_and_not_to_raise_a_missing_one() {
        let start = SHELL.find("\n## Language servers\n").expect("the section");
        let rest = &SHELL[start + 1..];
        let section = rest.find("\n## ").map_or(rest, |end| &rest[..end]);
        let words = section.split_whitespace().collect::<Vec<_>>().join(" ");
        for fact in [
            "starts when a file of that repository is first touched",
            "`No language servers configured for this project`",
            "then `lsp status` lists the servers",
            "the `PATH` it inherits, which is the **daemon's** environment",
            "A dev shell (`nix develop`, direnv) does not count",
            "`No language server found for this action`",
            "install the server where the daemon's `PATH` sees it",
            "declare it in `<workspace>/.omp/lsp.json` with an absolute `command`",
            "`fileTypes` and `rootMarkers`",
            "the `lsp` tool's `reload` action",
            "a missing server is not something to raise",
            "nor suggest installing or configuring one, unless the user asks",
            "Never install a server or change the user's system",
        ] {
            assert!(words.contains(fact), "{fact:?} missing: {words}");
        }
        assert!(
            index().contains("how omp finds language servers"),
            "{}",
            index()
        );
    }
}
