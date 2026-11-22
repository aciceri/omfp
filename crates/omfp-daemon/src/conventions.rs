//! What the daemon tells the agent about its workspace (DMN-030): a short text passed with
//! `--append-system-prompt` — the workspace conventions, then a note on widgets (AGT-024) and one on driving omfp
//! (AGT-045) — followed by the user's own `APPEND_SYSTEM.md`, which the flag would otherwise disable (S-10). Nothing
//! is written into the workspace for it.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::paths::{DataDir, replace_file};

/// The daemon's workspace conventions for the agent of a thread living in workspace `name`, `id` and `path`.
#[must_use]
pub fn workspace_conventions(name: &str, id: &str, path: &Path) -> String {
    let path = path.display();
    format!(
        "# omfp workspace\n\
         \n\
         You work in the omfp workspace \"{name}\" (ID: {id}), the folder {path} (your working directory). Its \
         repositories are its top-level entries; omfp finds them there and adds each one as a workspace root.\n\
         \n\
         - Work on a project from elsewhere: link it with `ln -s <project> {path}/<name>`.\n\
         - Work on a separate branch: `git -C <repo> worktree add {path}/<name> -b <branch>`.\n\
         - Get a remote repository: `git clone <url> {path}/<name>`.\n\
         - Keep repositories at the top level of the workspace: omfp does not look deeper."
    )
}

/// The user's append prompt as omp would load it: `<workspace>/.omp/APPEND_SYSTEM.md` if present, else
/// `<agent dir>/APPEND_SYSTEM.md`; trimmed, `None` when absent or empty.
#[must_use]
pub fn user_append(workspace: &Path, agent_dir: Option<&Path>) -> Option<String> {
    let project = workspace.join(".omp").join("APPEND_SYSTEM.md");
    let file = if project.is_file() {
        project
    } else {
        agent_dir?.join("APPEND_SYSTEM.md")
    };
    let text = std::fs::read_to_string(file).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// Where widgets live and what they may import, shared by the widgets note and the `widget-builder` agent so the two
/// never drift: `root` prefixes the workspace paths (`<workspace>/` for the main agent, empty for the subagent, whose
/// cwd is the workspace folder).
fn widget_facts(root: &str, user_library: &Path) -> String {
    let library = user_library.display();
    format!(
        "- Workspace widgets: {root}.omfp/widgets/<name>/index.tsx, used as `./widgets/<name>`; a folder named like a \
         built-in (`chat` for `omfp:chat`) replaces it here.\n\
         - User library: {library}/<name>/, used as `user:<name>` in every workspace.\n\
         - Built-in widgets' sources: omfp://widgets/builtin and omfp://widgets/builtin/<name> (never search the disk).\n\
         - Imports: only react, react/jsx-runtime, react-dom, omfp/sdk, omfp/kit and files of the widget's folder. A \
         saved widget reloads at once; errors show in its place. Controls come from omfp/kit (Select, Checkbox, \
         RadioGroup, TextField, SearchField), never native <select>, <input type=checkbox|radio|range|date|color> or \
         <datalist>: their popups use the platform's look, not the theme.\n\
         - useJob(command) runs `sh -c <command>` in the workspace folder (not a repository) with the daemon's \
         environment: no login shell, no dev shell (write `nix develop -c …` yourself), no timeout. For a command built \
         from input, pass `useJob(command, {{ key }})` so every run replaces the last. Details: omfp://docs/widgets (Jobs)."
    )
}

/// A ten-line example widget, shared like [`widget_facts`].
const EXAMPLE: &str = "```tsx\n\
    import { Card, Log, Progress } from \"omfp/kit\";\n\
    import { useJob } from \"omfp/sdk\";\n\
    export default function Tests({ command = \"make test\" }: { command?: string }) {\n\
    \x20 const job = useJob(command); // status: pending | running | succeeded | failed; kept across reloads\n\
    \x20 return (\n\
    \x20   <Card title={`Tests: ${job.status}`}>\n\
    \x20     <Progress label=\"Tests\" value={job.status === \"running\" ? undefined : 1} max={1} />\n\
    \x20     <Log lines={job.lines} />\n\
    \x20   </Card>);\n\
    }\n\
    ```";

/// Name of the daemon's subagent that builds widgets (AGT-029).
pub const WIDGET_BUILDER: &str = "widget-builder";

/// The `main` slot rule (AGT-028), word for word in the widgets note, `omfp_ui_mount`'s description and
/// `omfp://docs/widgets`.
pub const MAIN_SLOT_RULE: &str = "Keep the main slot, `[\"tabs\", {\"id\": \"main\"}, …\"omfp:chat\"]`, in every \
    view: the files the user opens become its tabs (without it they float in a panel).";

/// How an agent places a widget (AGT-061, AGT-063), word for word in the widgets note and `omfp://docs/widgets`.
pub const PLACEMENT_RULE: &str = "Place a widget with omfp_ui_mount { id, widget | code, props?, view?, place?, \
    panel? } (`place`: `side_tab`, the default, `main_tab` or `end`), never by rewriting views.json: mounting an id \
    again replaces it, omfp_ui_update { id, props } drives it, omfp_ui_remove { id } removes it.";

/// The verification loop (AGT-084), word for word in the widgets note and `omfp://docs/widgets`.
pub const VERIFY_LOOP: &str = "Verify in the user's window: write → omfp_widget_check { name } (it renders a widget \
    no view shows out of sight) → omfp_windows (each window's views and where each comes from, added commands, \
    errors) → omfp_window_run { command: \"view.switch\", args: [\"<view>\"] } to show a view → omfp_windows again \
    for its errors.";

/// The widgets note (AGT-024): where the views manifest, the workspace's widgets and the user library (`user_library`)
/// live, the host URIs of the built-in sources, what a widget may import, the rules for the `main` slot, placing a
/// widget and checking it, where declared jobs live (DMN-331, AGT-091), how to delegate building one to the
/// `widget-builder` agent (AGT-029), when to notify the user (AGT-032), an example, and where to read more.
#[must_use]
pub fn widgets_note(path: &Path, user_library: &Path) -> String {
    let display = path.display();
    format!(
        "# omfp widgets\n\
         \n\
         The user's omfp window shows this workspace through React widgets that you can write and arrange too.\n\
         - Views: {display}/.omfp/views.json (schema omfp://schemas/views.json), the user's and yours to change. \
         {MAIN_SLOT_RULE} A view may instead be a TSX file, `{{ \"view\": \"./views/<name>.tsx\" }}` for \
         {display}/.omfp/views/<name>.tsx (see omfp://docs/widgets); it takes no mounts: edit the file.\n\
         {facts}\n\
         - {PLACEMENT_RULE} omfp_component_save / omfp_component_list keep widgets for every workspace \
         (`user:<name>`). {VERIFY_LOOP} omfp_widget_check also checks init files (`init`, `user:init`) and TSX views \
         (`views/<file>`); omfp_views_errors lists the runtime errors windows show for them. omfp_window_run runs \
         only registered commands, never code.\n\
         - Cards: content of one moment of the conversation (a terminal to show something, a chart, a question with \
         its own controls) goes in a card, omfp_card {{ widget | code, props?, title?, wait? }}, drawn in your tool \
         call in the chat (with wait: true the user's answer is its result); content that stays goes in a mount \
         (omfp_ui_mount), a tab of every window's layout until it is removed or the user closes it. Details: \
         omfp://docs/widgets (Cards in the chat).\n\
         - Keys: {display}/.omfp/keymap.json (schema omfp://schemas/keymap.json) binds keys to commands, built-in or a \
         widget's useCommand, in windows showing this workspace; give a widget a command and bind it there.\n\
         - Jobs: a workspace's recurring commands, schedules, watchers and prompts are declared in \
         {display}/.omfp/jobs.json (schema omfp://schemas/jobs.json; details in omfp://docs/widgets, Declared jobs).\n\
         - Notify the user with omfp_notify {{ title, body }} when a long task finished or you need them; once per \
         occasion, never for progress.\n\
         - When something would read better as a visualization or controls (progress, a table, a chart, buttons), \
         propose a widget in one sentence and build it only after the user agrees.\n\
         - Once the user agrees, delegate the building: a `task` with `agent: \"{WIDGET_BUILDER}\"` and a complete brief \
         (what to show, its data sources, where it goes); keep talking, and place the widget with omfp_ui_mount when \
         the task's result arrives.\n\
         \n\
         {EXAMPLE}\n\
         \n\
         Before writing widgets read omfp://docs/widgets; schema omfp://schemas/views.json. omfp://docs indexes the \
         docs: omfp://docs/shell (the window and its `:` commands), omfp://keymap (its keys and commands).",
        facts = widget_facts(&format!("{display}/"), user_library),
    )
}

/// The `widget-builder` agent definition (AGT-029), an omp agent file: frontmatter (its tools, the user's `smol` model
/// role, not blocking so the main agent keeps talking) and a body carrying the widgets knowledge, since a subagent gets
/// neither the main agent's appended prompt nor its host tools (S-14).
#[must_use]
pub fn widget_builder_definition(user_library: &Path) -> String {
    format!(
        "---\n\
         name: {WIDGET_BUILDER}\n\
         description: Builds or changes one omfp widget in the workspace and checks it until it is clean. Give it a \
         complete brief - what to show, its data sources, where it goes; place the widget yourself afterwards.\n\
         tools: read, write, edit, glob, grep\n\
         model: \"@smol\"\n\
         ---\n\
         # omfp widget builder\n\
         \n\
         You build or change one React widget of the user's omfp window, as the brief asks. Your working directory is \
         the workspace folder: use relative paths.\n\
         {facts}\n\
         - First read omfp://docs/widgets (the SDK and kit reference) and, for how views use widgets, \
         omfp://schemas/views.json; a built-in close to what you need is a good start (omfp://widgets/builtin/<name>).\n\
         - After every write or edit, read omfp://widgets/check/<name> (`user:<name>` for the user library): it \
         compiles the widget and says what open windows render. Fix what it reports and read it again until it is clean.\n\
         - Never edit .omfp/views.json: the main agent places the widget.\n\
         - Finish with a short summary naming the widget (`./widgets/<name>` or `user:<name>`) and its props.\n\
         \n\
         {EXAMPLE}\n",
        facts = widget_facts("", user_library),
    )
}

/// Writes the `widget-builder` agent (AGT-029) to `data.widget_builder()`, inside the omp extension root the daemon
/// passes with `--extension` (DMN-107), unless the file already holds this content: returns whether it wrote.
pub fn write_widget_builder(data: &DataDir) -> std::io::Result<bool> {
    let path = data.widget_builder();
    let content = widget_builder_definition(&data.widgets());
    if std::fs::read_to_string(&path).is_ok_and(|current| current == content) {
        return Ok(false);
    }
    replace_file(&path, content.as_bytes(), None)?;
    Ok(true)
}

/// The control note (AGT-045): the thread's own id, `omfp_call` over every protocol method and its catalogue,
/// `thread.session` for a thread's session facts (AGT-085), `omfp_thread_wait`, and that new MCP tools arrive without
/// a restart (AGT-080). The widget tools are the widgets note's.
#[must_use]
pub fn control_note(thread_id: &str) -> String {
    format!(
        "# omfp control\n\
         \n\
         You can drive omfp as the user's windows do, through every protocol method; your thread id is {thread_id}.\n\
         - {call} {{ method, params }} calls one, e.g. threads.list, threads.create {{ workspace, title, prompt }}, \
         thread.prompt; {methods} lists them all, {methods}/<name> gives one's params and result schemas. For a \
         thread's session id, session file, model, state or cost call thread.session {{ threadId }}: it is small and \
         never starts omp, while thread.state is omp's whole state (tool definitions, system prompt).\n\
         - {wait} {{ threadId, promptId? }} waits until another thread is done or needs the user and answers its \
         last reply: use it instead of polling.\n\
         - MCP servers added (`/mcp add`, an MCP config file) or reauthorized (`/mcp reauth`) give you their tools \
         from your next turn: omfp reloads your omp once you are idle, so never ask the user to restart.",
        call = crate::agent_tools::CALL,
        wait = crate::agent_tools::THREAD_WAIT,
        methods = crate::agent_tools::catalog::METHODS_URL,
    )
}

/// What the daemon itself appends to the system prompt of thread `thread_id`: the workspace conventions, the widgets
/// note, then the control note.
#[must_use]
pub fn daemon_text(
    name: &str,
    id: &str,
    path: &Path,
    user_library: &Path,
    thread_id: &str,
) -> String {
    format!(
        "{}\n\n{}\n\n{}",
        workspace_conventions(name, id, path),
        widgets_note(path, user_library),
        control_note(thread_id)
    )
}

/// The `--append-system-prompt` text of thread `thread_id`: the daemon's text, then the user's append prompt when
/// there is one.
#[must_use]
pub fn append_system_prompt(
    name: &str,
    id: &str,
    path: &Path,
    agent_dir: Option<&Path>,
    user_library: &Path,
    thread_id: &str,
) -> String {
    let text = daemon_text(name, id, path, user_library, thread_id);
    match user_append(path, agent_dir) {
        Some(user) => format!("{text}\n\n{user}"),
        None => text,
    }
}

/// The home omp runs with: `HOME` of the omp environment overrides when they set it, else `home`.
#[must_use]
pub fn omp_home(
    overrides: &[(OsString, Option<OsString>)],
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    let home = match overrides.iter().rev().find(|(key, _)| key == "HOME") {
        Some((_, value)) => value.as_deref(),
        None => home,
    }?;
    Some(PathBuf::from(home))
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: DMN-030
    #[test]
    fn conventions_name_the_workspace_and_how_to_bring_repositories_in() {
        let text = workspace_conventions("checkout", "01WORKSPACE", Path::new("/ws/checkout"));
        assert!(text.starts_with("# omfp workspace\n"), "{text}");
        assert!(
            text.contains("\"checkout\" (ID: 01WORKSPACE), the folder /ws/checkout"),
            "{text}"
        );
        for rule in [
            "top-level entries",
            "ln -s <project> /ws/checkout/<name>",
            "worktree add /ws/checkout/<name>",
            "git clone <url> /ws/checkout/<name>",
            "does not look deeper",
        ] {
            assert!(text.contains(rule), "{rule:?} missing: {text}");
        }
        assert!(text.lines().count() <= 10, "short: {text}");
        assert!(!text.ends_with('\n'));
    }

    // spec: AGT-024, AGT-028, AGT-029, AGT-031, AGT-032, AGT-074, AGT-075, AGT-076, AGT-084
    #[test]
    fn the_widgets_note_says_where_widgets_live_what_they_import_and_how_to_check_and_place_them() {
        let library = Path::new("/data/omfp/widgets");
        let text = widgets_note(Path::new("/ws/checkout"), library);
        assert!(text.starts_with("# omfp widgets\n"), "{text}");
        for rule in [
            "/ws/checkout/.omfp/views.json",
            "/ws/checkout/.omfp/widgets/<name>/index.tsx",
            "`./widgets/<name>`",
            "/data/omfp/widgets/<name>/",
            "`user:<name>`",
            "omfp://widgets/builtin/<name>",
            "react, react/jsx-runtime, react-dom, omfp/sdk, omfp/kit",
            "omfp_widget_check { name }",
            PLACEMENT_RULE,
            MAIN_SLOT_RULE,
            VERIFY_LOOP,
            "omfp_component_save / omfp_component_list",
            "propose a widget in one sentence and build it only after the user agrees",
            "a `task` with `agent: \"widget-builder\"` and a complete brief \
             (what to show, its data sources, where it goes); keep talking, and place the widget with omfp_ui_mount \
             when the task's result arrives",
            "import { Card, Log, Progress } from \"omfp/kit\";",
            "useJob(command)",
            "job.status",
            "read omfp://docs/widgets; schema omfp://schemas/views.json",
            "- Keys: /ws/checkout/.omfp/keymap.json (schema omfp://schemas/keymap.json) binds keys to commands",
            "- Notify the user with omfp_notify { title, body } when a long task finished or you need them; once per \
             occasion, never for progress.\n",
            "`{ \"view\": \"./views/<name>.tsx\" }` for /ws/checkout/.omfp/views/<name>.tsx (see omfp://docs/widgets)",
            "omfp_widget_check also checks init files (`init`, `user:init`) and TSX views (`views/<file>`)",
            "omfp_views_errors lists the runtime errors windows show for them",
            "omfp_window_run runs only registered commands, never code.",
            "omfp://docs indexes the docs: omfp://docs/shell (the window and its `:` commands), omfp://keymap (its keys \
             and commands)",
        ] {
            assert!(text.contains(rule), "{rule:?} missing: {text}");
        }
        assert!(
            !text.contains("builtin-widgets") && !text.contains("absent"),
            "no bundle path, and views.json is always there: {text}"
        );
        let example = text.split("```").nth(1).unwrap();
        assert!(
            example.lines().count() <= 11,
            "a ten-line example: {example}"
        );
        assert!(text.lines().count() <= 31, "short: {text}");
        assert!(!text.ends_with('\n'));
    }

    // spec: DMN-030, AGT-024, AGT-045
    #[test]
    fn the_users_append_file_follows_the_conventions_and_the_notes_and_the_workspace_one_wins() {
        let ws = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let library = Path::new("/data/widgets");
        let prompt = |agent: &Path| {
            append_system_prompt(
                "w",
                "01WORKSPACE",
                ws.path(),
                Some(agent),
                library,
                "01THREAD",
            )
        };
        let daemon = format!(
            "{}\n\n{}\n\n{}",
            workspace_conventions("w", "01WORKSPACE", ws.path()),
            widgets_note(ws.path(), library),
            control_note("01THREAD")
        );
        assert_eq!(
            daemon_text("w", "01WORKSPACE", ws.path(), library, "01THREAD"),
            daemon
        );
        assert_eq!(prompt(agent.path()), daemon);

        std::fs::write(agent.path().join("APPEND_SYSTEM.md"), "  user rule\n").unwrap();
        assert_eq!(prompt(agent.path()), format!("{daemon}\n\nuser rule"));

        std::fs::create_dir(ws.path().join(".omp")).unwrap();
        std::fs::write(ws.path().join(".omp/APPEND_SYSTEM.md"), "project rule").unwrap();
        assert_eq!(prompt(agent.path()), format!("{daemon}\n\nproject rule"));
    }

    // spec: AGT-045
    #[test]
    fn the_control_note_names_the_thread_omfp_call_its_catalogue_and_the_wait() {
        let text = control_note("01THREAD");
        assert!(text.starts_with("# omfp control\n"), "{text}");
        for rule in [
            "your thread id is 01THREAD",
            "omfp_call { method, params }",
            "threads.list",
            "threads.create",
            "thread.prompt",
            "omfp://protocol/methods lists them all",
            "omfp://protocol/methods/<name>",
            "omfp_thread_wait { threadId, promptId? }",
            "instead of polling",
        ] {
            assert!(text.contains(rule), "{rule:?} missing: {text}");
        }
        assert!(
            !text.contains("omfp_ui_"),
            "the widget tools are the widgets note's: {text}"
        );
        assert!(text.lines().count() <= 6, "short: {text}");
        assert!(!text.ends_with('\n'));
    }

    // spec: AGT-024, AGT-028, AGT-084, AGT-061
    #[test]
    fn the_note_the_docs_and_the_mount_tool_state_the_rules_word_for_word() {
        let note = widgets_note(Path::new("/ws"), Path::new("/data/widgets"));
        // Markdown wraps lines and sets code in backticks: compare single spaces, without backticks.
        let plain = |text: &str| {
            text.replace('`', "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        let docs = plain(crate::widgets::DOCS);
        for rule in [MAIN_SLOT_RULE, PLACEMENT_RULE, VERIFY_LOOP] {
            assert_eq!(note.matches(rule).count(), 1, "{rule:?} once in: {note}");
            assert!(
                docs.contains(&plain(rule)),
                "{rule:?} missing from omfp://docs/widgets"
            );
        }
        let registry = crate::agent_tools::registry();
        let mount = registry
            .iter()
            .find(|tool| tool.name == crate::agent_tools::mounts::UI_MOUNT)
            .unwrap();
        assert!(
            mount.description.contains(MAIN_SLOT_RULE),
            "{}",
            mount.description
        );
    }

    // spec: AGT-080
    #[test]
    fn the_control_note_says_new_mcp_tools_arrive_without_a_restart() {
        let text = control_note("01THREAD");
        for rule in [
            "/mcp add",
            "/mcp reauth",
            "from your next turn",
            "never ask the user to restart",
        ] {
            assert!(text.contains(rule), "{rule:?} missing: {text}");
        }
    }

    // spec: AGT-085
    #[test]
    fn the_control_note_points_to_thread_session_for_a_threads_session_facts() {
        let text = control_note("01THREAD");
        for rule in [
            "session id, session file, model, state or cost call thread.session { threadId }",
            "never starts omp",
            "thread.state is omp's whole state",
        ] {
            assert!(text.contains(rule), "{rule:?} missing: {text}");
        }
    }

    // spec: AGT-029
    #[test]
    fn the_widget_builder_is_a_background_smol_agent_carrying_the_widgets_facts() {
        let library = Path::new("/data/omfp/widgets");
        let text = widget_builder_definition(library);
        let (frontmatter, body) = text
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n"))
            .expect("frontmatter");
        let fields: Vec<&str> = frontmatter
            .lines()
            .map(|line| line.split_once(": ").unwrap().0)
            .collect();
        assert_eq!(fields, ["name", "description", "tools", "model"]);
        for field in [
            "name: widget-builder",
            "tools: read, write, edit, glob, grep",
            "model: \"@smol\"",
        ] {
            assert!(
                frontmatter.lines().any(|line| line == field),
                "{field}: {frontmatter}"
            );
        }
        assert!(!frontmatter.contains("blocking"), "runs in the background");
        assert!(
            body.contains(&widget_facts("", library)),
            "the same facts as the widgets note, with relative paths: {body}"
        );
        assert!(widgets_note(Path::new("/ws"), library).contains(&widget_facts("/ws/", library)));
        for rule in [
            "working directory is the workspace folder: use relative paths",
            "- Workspace widgets: .omfp/widgets/<name>/index.tsx",
            "read omfp://docs/widgets",
            "omfp://widgets/builtin/<name>",
            "omfp://schemas/views.json",
            "react, react/jsx-runtime, react-dom, omfp/sdk, omfp/kit and files of the widget's folder",
            "After every write or edit, read omfp://widgets/check/<name>",
            "until it is clean",
            "Never edit .omfp/views.json",
            "short summary naming the widget (`./widgets/<name>` or `user:<name>`) and its props",
            EXAMPLE,
        ] {
            assert!(body.contains(rule), "{rule:?} missing: {body}");
        }
        assert!(
            !body.contains("omfp_"),
            "no tools of the main agent in a subagent: {body}"
        );
    }

    // spec: AGT-087
    #[test]
    fn the_widgets_note_and_the_builder_say_how_jobs_run_and_when_to_key_them() {
        let library = Path::new("/data/omfp/widgets");
        for text in [
            widgets_note(Path::new("/ws"), library),
            widget_builder_definition(library),
        ] {
            for rule in [
                "useJob(command) runs `sh -c <command>` in the workspace folder (not a repository)",
                "the daemon's environment: no login shell, no dev shell (write `nix develop -c …` yourself), no timeout",
                "`useJob(command, { key })` so every run replaces the last",
                "omfp://docs/widgets (Jobs)",
            ] {
                assert!(text.contains(rule), "{rule:?} missing: {text}");
            }
        }
    }

    // spec: DMN-331, AGT-091
    #[test]
    fn the_widgets_note_names_the_jobs_file_and_its_schema() {
        let text = widgets_note(Path::new("/ws/checkout"), Path::new("/data/omfp/widgets"));
        assert!(
            text.contains(
                "- Jobs: a workspace's recurring commands, schedules, watchers and prompts are declared in \
                 /ws/checkout/.omfp/jobs.json (schema omfp://schemas/jobs.json; details in omfp://docs/widgets, \
                 Declared jobs).\n"
            ),
            "{text}"
        );
        assert_eq!(text.matches("jobs.json").count(), 2, "one line: {text}");
    }

    // spec: AGT-091
    #[test]
    fn the_widgets_docs_declare_jobs_for_agents() {
        let docs = crate::widgets::DOCS;
        let section = docs
            .split_once("\n## Declared jobs\n")
            .and_then(|(_, rest)| rest.split("\n## ").next())
            .expect("a Declared jobs section");
        for needle in [
            ".omfp/jobs.json",
            "omfp://schemas/jobs.json",
            "\"trigger\": \"schedule\"",
            "\"thread\": \"reuse\"",
            "\"trigger\": \"service\"",
            "\"restart\": \"on-failure\"",
            "\"cwd\":",
            "a `schedule` job with a `prompt`",
            "local time zone",
            "**One run per job**",
            "**Missed times**",
            "stays stopped",
            "see Jobs above",
            "leaves them running",
            "50 newest finished runs",
            "another daemon",
            "jobs.list",
            "jobs.runs",
            "jobs.run",
            "jobs.define",
            "jobs.remove",
            "jobs.enable",
            "jobs.kill",
            "jobs/<workspaceId>",
            "useJobs",
            "omfp.jobs",
            "job_started",
            "job_ended",
            "job_failed",
            "`job.run <name>` (`:job-run`)",
            "[\"job.run\", \"deploy\"]",
            "An init file's listener runs once per open window, and never when no window is open: it may notify or \
             show\nsomething, but must not start runs or chain jobs",
        ] {
            assert!(
                section.contains(needle),
                "{needle:?} missing from Declared jobs"
            );
        }
        for row in [
            "| `useJobs(workspaceId?)` |",
            "| `jobs` | The current workspace's declared jobs",
        ] {
            assert!(docs.contains(row), "{row:?} missing from the tables");
        }
        assert!(
            docs.contains("start *transient runs*"),
            "the Jobs section names transient runs"
        );
    }

    // spec: AGT-088
    #[test]
    fn the_widgets_note_and_the_builder_send_controls_to_the_kit() {
        let library = Path::new("/data/omfp/widgets");
        for text in [
            widgets_note(Path::new("/ws"), library),
            widget_builder_definition(library),
        ] {
            for rule in [
                "Controls come from omfp/kit (Select, Checkbox, RadioGroup, TextField, SearchField)",
                "never native <select>, <input type=checkbox|radio|range|date|color> or <datalist>",
                "their popups use the platform's look, not the theme",
            ] {
                assert!(text.contains(rule), "{rule:?} missing: {text}");
            }
        }
    }

    // spec: AGT-098
    #[test]
    fn the_widgets_note_gives_one_line_to_cards_for_content_of_one_moment() {
        let text = widgets_note(Path::new("/ws"), Path::new("/data/omfp/widgets"));
        let cards: Vec<&str> = text
            .lines()
            .filter(|line| line.contains("omfp_card"))
            .collect();
        assert_eq!(
            cards,
            [
                "- Cards: content of one moment of the conversation (a terminal to show something, a chart, a \
                 question with its own controls) goes in a card, omfp_card { widget | code, props?, title?, wait? }, \
                 drawn in your tool call in the chat (with wait: true the user's answer is its result); content that \
                 stays goes in a mount (omfp_ui_mount), a tab of every window's layout until it is removed or the \
                 user closes it. Details: omfp://docs/widgets (Cards in the chat)."
            ]
        );
    }

    // spec: AGT-029, DMN-107
    #[test]
    fn the_widget_builder_is_written_under_the_data_dir_and_rewritten_only_when_it_changed() {
        let root = tempfile::tempdir().unwrap();
        let data = DataDir::new(root.path());
        let path = root.path().join("omp/agents/widget-builder.md");
        let expected = widget_builder_definition(&root.path().join("widgets"));

        assert!(write_widget_builder(&data).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
        let entries: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["omp"], "nothing else written");

        assert!(!write_widget_builder(&data).unwrap(), "same content");

        std::fs::write(&path, "an older definition").unwrap();
        assert!(write_widget_builder(&data).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
    }
}
