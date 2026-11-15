//! The critical journeys (06-testing.md, "End-to-end scenarios") against the TypeScript UI, each in a fresh browser,
//! daemon, omp environment and fake LLM. Run with `just test-e2e`.

use e2e::{Journey, row, system_prompt, workspace_roots};
use fake_llm::{Reply, Step, ToolCall};
use omfp_testkit::TempRepo;
use serde_json::json;
use thirtyfour::prelude::*;

fn repository(marker: &str) -> TempRepo {
    let repo = TempRepo::new().unwrap();
    repo.commit_file(
        "AGENTS.md",
        &format!("Marker: {marker}\n"),
        "add agents file",
    )
    .unwrap();
    repo
}

/// The header's chips (UI-340: the workspace's only) as `(text, tooltip)`, the tooltip listing the repositories.
async fn header_chips(journey: &Journey) -> anyhow::Result<Vec<(String, String)>> {
    let css = "[data-testid=thread-header] [data-testid=workspace-chip]";
    let texts = journey.texts(css).await?;
    let tooltips = journey.attrs(&format!("{css} [title]"), "title").await?;
    Ok(texts
        .into_iter()
        .zip(tooltips.into_iter().map(Option::unwrap_or_default))
        .collect())
}

// spec: E2E-01, UI-002, UI-023, UI-340, TST-031
#[tokio::test(flavor = "multi_thread")]
async fn a_draft_thread_in_a_workspace_linking_a_repository_is_created_by_its_first_message()
-> anyhow::Result<()> {
    let repo = repository("kumquat-17");
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_1",
            "bash",
            json!({ "command": "pwd && cat */AGENTS.md" }),
        )])),
        Step::reply(Reply::text("Hello from the linked repository.")),
    ])
    .await?;
    journey.connected().await?;
    // The token left the address bar (UI-002).
    let url = journey.driver.current_url().await?;
    assert_eq!(url.fragment(), None, "{url}");

    // "+ workspace" with a repository's path, picked from the daemon's completions, makes a workspace holding a link
    // to it and opens a draft thread there; the first message creates the thread (UI-023, DMN-021).
    journey
        .new_linked_workspace(&partial(repo.path()), &repo.path().to_string_lossy())
        .await?;
    assert_eq!(
        journey.thread_ids().await?,
        Vec::<String>::new(),
        "a draft is no thread yet"
    );
    journey
        .element("[data-testid=workspace-repo][data-kind=link]")
        .await?;
    let thread = journey.first_message("hello").await?;

    // One workspace chip, named after the repository's folder with its one repository, the link in its tooltip.
    let name = dir_name(&repo);
    let chips = journey
        .until("the workspace chip counting the link", async |page| {
            let chips = header_chips(page).await?;
            Ok(chips
                .iter()
                .any(|(text, _)| text.ends_with("1 repo"))
                .then_some(chips))
        })
        .await?;
    assert_eq!(chips.len(), 1, "{chips:?}");
    assert!(chips[0].0.contains(&name), "{chips:?}");
    assert!(chips[0].1.contains(&name), "{chips:?}");
    assert!(chips[0].1.contains("(link)"), "{chips:?}");

    journey.reply("Hello from the linked repository.").await?;
    journey.status(&thread, "idle").await?;
    // The streamed reply is rendered as Markdown, and the first message is the thread's placeholder title.
    assert_eq!(
        journey
            .count("[data-testid=message][data-role=assistant] [data-testid=markdown]")
            .await?,
        1
    );
    assert_eq!(
        journey.attr(&row(&thread), "data-title-source").await?,
        Some("placeholder".to_owned())
    );
    // The agent runs in the workspace folder, made under the workspaces root, the linked repository inside it.
    let request = journey.fake.requests()[1].to_string();
    assert!(
        request.contains("/workspaces/"),
        "the cwd is the workspace folder: {request}"
    );
    assert!(
        request.contains("kumquat-17"),
        "the link is an entry of the workspace folder"
    );
    journey.finish("E2E-01").await
}

/// Clicks the option of the pending (unanswered) request whose text (omp's option label) contains `needle`.
async fn choose(journey: &Journey, needle: &str) -> anyhow::Result<()> {
    let clicked = journey
        .eval(
            "const option = [...document.querySelectorAll(
                 '[data-testid=interaction]:not([aria-busy=true]) [data-testid=interaction-option]')]
               .find(e => e.innerText.includes(arguments[0]));
             if (option) { option.click(); }
             return !!option;",
            vec![json!(needle)],
        )
        .await?;
    assert_eq!(clicked, json!(true), "no option containing {needle:?}");
    Ok(())
}

/// The heading of the pending request, once one shows.
async fn pending_request(journey: &Journey) -> anyhow::Result<Option<String>> {
    journey
        .attr(
            "[data-testid=interaction]:not([aria-busy=true])",
            "aria-label",
        )
        .await
}

/// Waits until the transcript shows exactly one tool card, in `status`.
async fn one_tool_card(journey: &Journey, status: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("the tool card to be {status}"), async |page| {
            let statuses = page.attrs("[data-testid=tool-card]", "data-status").await?;
            Ok((statuses == vec![Some(status.to_owned())]).then_some(()))
        })
        .await
}

// spec: E2E-02, UI-070, UI-092
#[tokio::test(flavor = "multi_thread")]
async fn a_tool_needing_approval_completes_after_approving_it_in_the_ui() -> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_1",
            "bash",
            json!({ "command": "echo approved-run" }),
        )])),
        Step::reply(Reply::text("Done after approval.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_asking_thread("approval journey").await?;
    journey.send("run it").await?;

    journey.status(&thread, "awaiting").await?;
    journey
        .until("the approval request", async |page| {
            pending_request(page).await
        })
        .await?;
    // The first option of the first pending request has the focus (UI-070); the arrow keys move between the options
    // (UI-092).
    journey.until_focused("interaction-option").await?;
    for (key, index) in [(Key::Right, 1), (Key::Left, 0)] {
        journey.keys(key).await?;
        journey
            .until(&format!("the focus on option {index}"), async |page| {
                let focused = page
                    .eval(
                        "return [...document.querySelectorAll('[data-testid=interaction-option]')]
                           .indexOf(document.activeElement);",
                        Vec::new(),
                    )
                    .await?;
                Ok((focused == json!(index)).then_some(()))
            })
            .await?;
    }
    choose(&journey, "Approve").await?;

    one_tool_card(&journey, "succeeded").await?;
    journey.reply("Done after approval.").await?;
    journey.status(&thread, "idle").await?;
    assert!(
        journey.fake.requests()[1]
            .to_string()
            .contains("approved-run")
    );
    journey.finish("E2E-02").await
}

const ASK_FORM: &str = r#"<form>
  <fieldset><legend>Which database?</legend>
    <label><input type="radio" name="db" value="postgres"> Postgres</label>
    <label><input type="radio" name="db" value="sqlite" checked> SQLite</label>
  </fieldset>
  <fieldset><legend>Which features?</legend>
    <label><input type="checkbox" name="features" value="auth"> Auth</label>
    <label><input type="checkbox" name="features" value="billing"> Billing</label>
    <label><input type="checkbox" name="features" value="search"> Search</label>
  </fieldset>
</form>"#;

// spec: E2E-03
#[tokio::test(flavor = "multi_thread")]
async fn an_ask_with_two_questions_is_answered_and_the_agent_gets_the_answers() -> anyhow::Result<()>
{
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_ask_1",
            "ask",
            json!({ "html": ASK_FORM }),
        )])),
        Step::reply(Reply::text("Using SQLite with Auth and Search.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("ask journey").await?;
    journey.send("set it up").await?;

    // omp walks the form with one `select` dialog per question; the multiple choice repeats until "Done".
    let mut headings = Vec::new();
    let mut features = ["Auth", "Search", "Done"].into_iter();
    loop {
        let next = journey
            .until("a question or the answer", async |page| {
                if let Some(heading) = pending_request(page).await? {
                    return Ok(Some(Some(heading)));
                }
                let replies = page
                    .texts("[data-testid=message][data-role=assistant]")
                    .await?;
                Ok(replies
                    .iter()
                    .any(|reply| reply.contains("Using SQLite"))
                    .then_some(None))
            })
            .await?;
        let Some(heading) = next else { break };
        let choice = if heading.contains("database") {
            "SQLite"
        } else {
            features.next().expect("at most three feature questions")
        };
        choose(&journey, choice).await?;
        headings.push(heading);
        // The answered card closes before the next request shows.
        journey
            .gone("[data-testid=interaction][aria-busy=true]")
            .await?;
    }
    assert!(
        headings
            .first()
            .is_some_and(|heading| heading.contains("Which database?")),
        "{headings:?}"
    );
    journey.status(&thread, "idle").await?;
    let second = journey.fake.requests()[1].to_string().to_lowercase();
    assert!(
        second.contains("sqlite") && second.contains("auth") && second.contains("search"),
        "the answers reach the model: {second}"
    );
    journey.finish("E2E-03").await
}

/// How many user and assistant messages and tool cards the transcript shows.
async fn transcript_counts(journey: &Journey) -> anyhow::Result<(usize, usize, usize)> {
    Ok((
        journey
            .count("[data-testid=message][data-role=user]")
            .await?,
        journey
            .count("[data-testid=message][data-role=assistant]")
            .await?,
        journey.count("[data-testid=tool-card]").await?,
    ))
}

// spec: E2E-04, UI-002
#[tokio::test(flavor = "multi_thread")]
async fn reloading_mid_turn_continues_the_transcript_without_gaps_or_duplicates()
-> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_1",
            "bash",
            json!({ "command": "echo before-reload" }),
        )])),
        Step::reply(Reply::text("Finished after the reload.")).gated(),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("reload journey").await?;
    journey.send("go").await?;
    journey.fake.wait_for_gate(1).await;
    one_tool_card(&journey, "succeeded").await?;
    let before = transcript_counts(&journey).await?;

    // The token comes from `localStorage` now: the address bar lost it on the first load (UI-002).
    journey.driver.refresh().await?;
    journey.connected().await?;
    journey.click(&row(&thread)).await?;
    journey.opened(&thread).await?;
    journey
        .until("the transcript after the reload", async |page| {
            Ok((transcript_counts(page).await? == before).then_some(()))
        })
        .await?;
    journey.status(&thread, "running").await?;

    journey.fake.release_gate(1);
    journey.reply("Finished after the reload.").await?;
    journey.status(&thread, "idle").await?;
    let (users, assistants, tools) = transcript_counts(&journey).await?;
    assert_eq!((users, tools), (before.0, before.2));
    assert_eq!(assistants, before.1 + 1, "the reply is shown once");
    let replies = journey
        .texts("[data-testid=message][data-role=assistant]")
        .await?;
    assert_eq!(
        replies
            .iter()
            .filter(|reply| reply.contains("Finished after the reload."))
            .count(),
        1
    );
    journey.finish("E2E-04").await
}

/// The composer's current text.
async fn draft(journey: &Journey) -> anyhow::Result<String> {
    let value = journey
        .eval(
            "return document.querySelector('[data-testid=composer-input]').value;",
            Vec::new(),
        )
        .await?;
    Ok(value.as_str().unwrap_or_default().to_owned())
}

/// The queued chips as `(queue, text)`.
async fn queue(journey: &Journey) -> anyhow::Result<Vec<(String, String)>> {
    let kinds = journey
        .attrs("[data-testid=queue-chip]", "data-queue")
        .await?;
    let texts = journey.texts("[data-testid=queue-chip]").await?;
    Ok(kinds
        .into_iter()
        .map(Option::unwrap_or_default)
        .zip(texts)
        .collect())
}

/// Waits until the queue shows a `kind` chip containing `text`.
async fn queued(journey: &Journey, kind: &str, text: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("the {kind} chip {text:?}"), async |page| {
            let chips = queue(page).await?;
            Ok(chips
                .iter()
                .any(|(queue, chip)| queue == kind && chip.contains(text))
                .then_some(()))
        })
        .await
}

/// Waits until the send button is in `mode`.
async fn send_mode(journey: &Journey, mode: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("the send button in {mode} mode"), async |page| {
            let modes = page.attrs("[data-testid=send]", "data-mode").await?;
            Ok((modes == vec![Some(mode.to_owned())]).then_some(()))
        })
        .await
}

// spec: E2E-05, UI-061, UI-062, UI-063, UI-095, UI-368
#[tokio::test(flavor = "multi_thread")]
async fn a_removed_follow_up_and_an_abort_bring_the_steer_back_to_the_composer()
-> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::text("Never sent.")).gated(),
        Step::reply(Reply::text("Never sent either.")).gated(),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("queue journey").await?;
    journey.send("start").await?;
    journey.fake.wait_for_gate(0).await;
    journey.status(&thread, "running").await?;
    journey.busy().await?;

    // Ctrl+Enter while busy queues a follow-up (UI-061), which can be removed (UI-062).
    journey.queue("later please").await?;
    queued(&journey, "follow-up", "later please").await?;
    journey.click("[data-testid=queue-remove]").await?;
    journey.gone("[data-testid=queue-chip]").await?;

    // Holding Shift in the composer switches the send button to queueing; leaving the composer while it is held
    // does not leave it stuck there.
    let input = journey.element("[data-testid=composer-input]").await?;
    input.click().await?;
    journey
        .driver
        .action_chain()
        .key_down(Key::Shift)
        .perform()
        .await?;
    send_mode(&journey, "follow-up").await?;
    journey.click("[data-testid=transcript]").await?;
    journey
        .driver
        .action_chain()
        .key_up(Key::Shift)
        .perform()
        .await?;
    send_mode(&journey, "steer").await?;

    // Alt+Enter is a new line, Enter steers (the trailing line break is trimmed).
    input.click().await?;
    input.send_keys("steer me").await?;
    journey
        .driver
        .action_chain()
        .key_down(Key::Alt)
        .send_keys(Key::Enter)
        .key_up(Key::Alt)
        .perform()
        .await?;
    assert_eq!(draft(&journey).await?, "steer me\n");
    input.send_keys(Key::Enter).await?;
    queued(&journey, "steering", "steer me").await?;
    assert_eq!(draft(&journey).await?, "");

    // Esc only leaves the composer: the turn keeps running with its queued steer (UI-063, UI-095).
    input.send_keys(Key::Escape).await?;
    journey
        .until("the focus out of the composer", async |page| {
            Ok((page.focused().await?.as_deref() != Some("composer-input")).then_some(()))
        })
        .await?;
    journey.status(&thread, "running").await?;
    queued(&journey, "steering", "steer me").await?;

    // Ctrl-C in normal mode aborts; the queued steer comes back into the composer (UI-368).
    journey
        .driver
        .action_chain()
        .key_down(Key::Control)
        .send_keys("c")
        .key_up(Key::Control)
        .perform()
        .await?;
    journey
        .until("the restored draft", async |page| {
            Ok(draft(page).await?.contains("steer me").then_some(()))
        })
        .await?;
    journey.status(&thread, "idle").await?;
    assert_eq!(queue(&journey).await?, Vec::new());

    // Ctrl-C clears a draft (UI-063): the restored one, with what was typed after it.
    input.send_keys(" and more").await?;
    input.send_keys(Key::Control + "c").await?;
    journey
        .until("the cleared draft", async |page| {
            Ok(draft(page).await?.is_empty().then_some(()))
        })
        .await?;
    journey.status(&thread, "idle").await?;

    // Busy again with a queued follow-up: Ctrl-C clears the half-typed draft (with its hint), the second one aborts
    // the turn and drops the queue without putting it back into the composer.
    journey.send("again").await?;
    journey.fake.wait_for_gate(1).await;
    journey.busy().await?;
    journey.queue("queued later").await?;
    queued(&journey, "follow-up", "queued later").await?;
    input.send_keys("half typed").await?;
    input.send_keys(Key::Control + "c").await?;
    journey.element("[data-testid=interrupt-hint]").await?;
    assert_eq!(draft(&journey).await?, "");
    input.send_keys(Key::Control + "c").await?;
    journey.status(&thread, "idle").await?;
    journey.gone("[data-testid=queue-chip]").await?;
    assert_eq!(draft(&journey).await?, "");
    journey.fake.release_gate(0);
    journey.fake.release_gate(1);
    journey.finish("E2E-05").await
}

/// Links the folder at `path` into the shown workspace from the Workspace widget; returns the entry name.
async fn link(journey: &Journey, path: &str) -> anyhow::Result<String> {
    let css = "[data-testid=workspace-repo][data-kind=link]";
    let before = journey.attrs(css, "data-name").await?;
    journey
        .click("[data-testid=add-repo-kind] [data-value=link]")
        .await?;
    journey
        .type_into("[data-testid=add-repo-path]", path)
        .await?;
    journey.click("[data-testid=add-repo-submit]").await?;
    new_entry(journey, css, &before).await
}

/// Waits until an entry matching `css` not named in `before` shows in the Workspace widget; returns its name.
async fn new_entry(
    journey: &Journey,
    css: &str,
    before: &[Option<String>],
) -> anyhow::Result<String> {
    journey
        .until("the new repository", async |page| {
            let names = page.attrs(css, "data-name").await?;
            Ok(names
                .into_iter()
                .find(|name| !before.contains(name))
                .flatten())
        })
        .await
}

/// `path` without its last three characters: typed into a path field, its completions include `path`.
fn partial(path: &std::path::Path) -> String {
    let path = path.to_string_lossy();
    path[..path.len() - 3].to_owned()
}

/// The entry name of the repository at `path`.
fn dir_name(repo: &TempRepo) -> String {
    repo.path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// Links `front` and `back`, two repositories in two different folders, each picked from the path field's
/// completions (DMN-021), into the shown workspace.
async fn link_two_by_completion(
    journey: &Journey,
    front: &TempRepo,
    back: &TempRepo,
) -> anyhow::Result<()> {
    let path = "[data-testid=add-repo-path]";
    let links = "[data-testid=workspace-repo][data-kind=link]";
    for repo in [back, front] {
        let before = journey.attrs(links, "data-name").await?;
        journey
            .pick_path(path, &partial(repo.path()), &repo.path().to_string_lossy())
            .await?;
        journey.click("[data-testid=add-repo-submit]").await?;
        new_entry(journey, links, &before).await?;
    }
    journey
        .until(
            "the workspace chip counting both repositories",
            async |page| {
                let chips = header_chips(page).await?;
                Ok(chips
                    .iter()
                    .any(|(text, _)| text.ends_with("2 repos"))
                    .then_some(()))
            },
        )
        .await
}

/// Replaces the left column's filter text with `query` and waits until thread `id` shows `rows` times.
async fn filter(journey: &Journey, query: &str, id: &str, rows: usize) -> anyhow::Result<()> {
    let field = journey.element("[data-testid=thread-filter]").await?;
    field.send_keys(Key::Control + "a").await?;
    field.send_keys(Key::Backspace).await?;
    if !query.is_empty() {
        field.send_keys(query).await?;
    }
    until_rows(journey, id, rows).await
}

// spec: E2E-08, UI-022, UI-023, UI-025, UI-092, DMN-021
#[tokio::test(flavor = "multi_thread")]
async fn repositories_picked_by_completion_make_the_thread_findable_by_their_names()
-> anyhow::Result<()> {
    let front = repository("front-1");
    let back = repository("back-2");
    let journey = Journey::start(vec![Step::reply(Reply::text("Ready."))]).await?;
    journey.connected().await?;

    // Space w (normal mode) focuses the "+ workspace" field; Enter opens a draft, the first message makes the thread.
    normal_mode(&journey).await?;
    journey.keys(" w").await?;
    journey.until_focused("new-workspace-input").await?;
    journey.keys("two repositories").await?;
    journey.keys(Key::Enter).await?;
    journey.element("[data-testid=draft-thread]").await?;
    journey.until_focused("composer-input").await?;
    let thread = journey.first_message("hello").await?;
    journey.reply("Ready.").await?;

    // Two links from two different folders, both picked from the completions.
    link_two_by_completion(&journey, &front, &back).await?;
    // The Workspace widget and its add control fit its column at a 1400 px window.
    journey.fits("[data-widget=\"omfp:workspace\"]").await?;

    // The column's persistent filter still finds repository names; `/` now searches the conversation.
    journey.click("[data-testid=thread-filter]").await?;
    journey.until_focused("thread-filter").await?;
    filter(&journey, &dir_name(&back), &thread, 1).await?;
    filter(&journey, &dir_name(&front).to_uppercase(), &thread, 1).await?;
    filter(&journey, "no such repository", &thread, 0).await?;
    journey.gone("[data-testid=workspace-section]").await?;
    filter(&journey, "", &thread, 1).await?;
    journey.finish("E2E-08").await
}

/// The selector of the Workspace panel's entry `name`.
fn entry(name: &str) -> String {
    format!("[data-testid=workspace-repo][data-name=\"{name}\"]")
}

// spec: E2E-09, UI-025, DMN-029
#[tokio::test(flavor = "multi_thread")]
async fn workspace_folder_entries_follow_the_ui_and_the_agent_live() -> anyhow::Result<()> {
    let linked = repository("apricot-31");
    let source = repository("quince-58");
    let target = repository("medlar-94");
    let command = format!(
        "git -C {} worktree add -b agent-branch \"$PWD/agent-wt\" && ln -s {} agent-link",
        source.path().display(),
        target.path().display(),
    );
    let journey = Journey::start(vec![
        Step::reply(Reply::text("Linked and ready.")),
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_1",
            "bash",
            json!({ "command": command }),
        )])),
        Step::reply(Reply::text("Made a worktree and a link.")),
        Step::reply(Reply::text("One repository less.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("folders journey").await?;

    // A link made in the Workspace panel shows there, and the next turn loads its AGENTS.md.
    let name = link(&journey, &linked.path().to_string_lossy()).await?;
    journey.send("hello").await?;
    journey.reply("Linked and ready.").await?;
    let system = system_prompt(&journey.fake.requests()[0]);
    assert!(
        system.contains("apricot-31"),
        "the link's AGENTS.md (roots {:?}): {system}",
        workspace_roots(&system)
    );
    let roots = workspace_roots(&system);
    let link_root = roots
        .iter()
        .find(|root| root.ends_with(&format!("/{name}")))
        .cloned()
        .unwrap_or_else(|| panic!("the link is a workspace root: {roots:?}"));

    // Entries the agent makes in the folder show up live.
    journey.send("add two more").await?;
    journey.reply("Made a worktree and a link.").await?;
    journey.element(&entry("agent-wt")).await?;
    journey.element(&entry("agent-link")).await?;
    assert_eq!(
        journey.attr(&entry("agent-wt"), "data-kind").await?,
        Some("worktree".to_owned())
    );
    assert_eq!(
        journey.attr(&entry("agent-link"), "data-kind").await?,
        Some("link".to_owned())
    );
    journey.status(&thread, "idle").await?;

    // Removing the link (no confirmation for a link) detaches it from the session with `/remove-dir`.
    journey
        .click(&format!("{} [data-testid=remove-repo]", entry(&name)))
        .await?;
    journey.gone(&entry(&name)).await?;
    journey.send("and now?").await?;
    journey.reply("One repository less.").await?;
    let system = system_prompt(&journey.fake.requests()[3]);
    let roots = workspace_roots(&system);
    assert!(
        !roots.contains(&link_root),
        "/remove-dir detached it: {roots:?}"
    );
    assert!(!system.contains("apricot-31"), "{system}");
    assert!(
        roots.iter().any(|root| root.ends_with("/agent-wt"))
            && roots.iter().any(|root| root.ends_with("/agent-link")),
        "the agent's entries were added: {roots:?}"
    );
    assert!(
        linked.path().join("AGENTS.md").is_file(),
        "the target stays"
    );
    journey.finish("E2E-09").await
}

/// Takes the focus out of every text field: the keymap's normal mode.
async fn normal_mode(journey: &Journey) -> anyhow::Result<()> {
    journey
        .eval(
            "if (document.activeElement) { document.activeElement.blur(); }",
            Vec::new(),
        )
        .await?;
    Ok(())
}

/// Presses `key` in normal mode and waits until thread `id` is selected.
async fn select_with(journey: &Journey, key: &str, id: &str) -> anyhow::Result<()> {
    journey.keys(key).await?;
    journey
        .until(&format!("{key} to select {id}"), async |page| {
            Ok((page.selected().await?.as_deref() == Some(id)).then_some(()))
        })
        .await
}

// spec: UI-095, UI-092
#[tokio::test(flavor = "multi_thread")]
async fn the_vim_keymap_moves_focuses_and_runs_the_command_line() -> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    journey.create_thread("first keys").await?;
    let second = journey.create_thread("second keys").await?;

    // j/k move the selection through the list in its order.
    normal_mode(&journey).await?;
    let order = journey.thread_ids().await?;
    assert_eq!(order.len(), 2, "{order:?}");
    let other = order.iter().find(|id| **id != second).unwrap().clone();
    let (forward, back) = if order[0] == second {
        ("j", "k")
    } else {
        ("k", "j")
    };
    select_with(&journey, forward, &other).await?;
    journey.opened(&other).await?;
    select_with(&journey, back, &second).await?;
    journey.opened(&second).await?;

    // i enters the composer; Esc leaves it.
    normal_mode(&journey).await?;
    journey.keys("i").await?;
    journey.until_focused("composer-input").await?;
    journey.keys(Key::Escape).await?;
    journey
        .until("the focus out of the composer", async |page| {
            Ok((page.focused().await?.as_deref() != Some("composer-input")).then_some(()))
        })
        .await?;

    // `:` opens the command line; `:theme dark` switches the colour scheme and closes it.
    normal_mode(&journey).await?;
    journey.keys(":").await?;
    journey.element("[data-testid=command-line]").await?;
    journey.until_focused("command-line-input").await?;
    journey.keys("theme dark").await?;
    journey.keys(Key::Enter).await?;
    journey.until_scheme("dark").await?;
    journey.gone("[data-testid=command-line]").await?;
    journey.finish("keymap").await
}

// spec: UI-091
#[tokio::test(flavor = "multi_thread")]
async fn a_theme_set_from_the_command_line_reaches_the_open_page_live() -> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    let built_in = journey.token("--omfp-primary").await?;
    assert!(!built_in.is_empty(), "the built-in theme sets the token");
    // A marker on `window` tells a reload apart from a live update.
    journey
        .eval("window.omfpE2eMarker = 'kept';", Vec::new())
        .await?;

    let dir = tempfile::tempdir()?;
    let file = dir.path().join("theme.css");
    std::fs::write(&file, ":root { --omfp-primary: #0b6e4f; }\n")?;
    let message = omfp::theme::run(omfp::theme::Command::Set(omfp::theme::SetArgs {
        file,
        daemon: omfp::remote::DaemonArgs {
            daemon: Some(format!("http://{}", journey.daemon().addr())),
            token: Some(journey.daemon().token().expose().to_owned()),
            data_dir: None,
        },
    }))
    .await?;
    assert!(message.starts_with("theme set from"), "{message}");

    journey
        .until("the pushed primary colour", async |page| {
            Ok((page.token("--omfp-primary").await? == "#0b6e4f").then_some(()))
        })
        .await?;
    let marker = journey
        .eval("return window.omfpE2eMarker || null;", Vec::new())
        .await?;
    assert_eq!(marker, json!("kept"), "the page did not reload");

    // Clearing it brings the built-in theme back, live too.
    let mut connection = journey.connect().await?;
    connection
        .call::<omfp_protocol::method::ThemeSet>(omfp_protocol::Theme { css: None })
        .await?;
    connection.close().await?;
    journey
        .until("the built-in primary colour", async |page| {
            Ok((page.token("--omfp-primary").await? == built_in).then_some(()))
        })
        .await?;
    journey.finish("live theme").await
}

/// Waits until `<html>` carries `data-theme="<theme>"` (UI-510).
async fn until_theme(journey: &Journey, theme: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("the {theme} theme"), async |page| {
            let value = page.attr("html", "data-theme").await?;
            Ok((value.as_deref() == Some(theme)).then_some(()))
        })
        .await
}

/// The built-in themes with their official background in light and dark (`--omfp-surface`, UI-511).
const THEME_SURFACES: [(&str, &str, &str); 7] = [
    ("synth", "#faf8ff", "#12131a"),
    ("catppuccin", "#eff1f5", "#1e1e2e"),
    ("gruvbox", "#fbf1c7", "#282828"),
    ("tokyo-night", "#e1e2e7", "#1a1b26"),
    ("nord", "#eceff4", "#2e3440"),
    ("solarized", "#fdf6e3", "#002b36"),
    ("rose-pine", "#faf4ed", "#191724"),
];

// spec: E2E-44, UI-510, UI-511, UI-512, UI-513, DMN-239
#[tokio::test(flavor = "multi_thread")]
async fn a_built_in_theme_is_chosen_with_theme_kept_by_the_daemon_and_under_the_user_theme()
-> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    until_theme(&journey, "synth").await?;

    // `:theme <theme> <scheme>`: the theme goes to the config file, the scheme stays the window's.
    journey.command("theme catppuccin dark").await?;
    until_theme(&journey, "catppuccin").await?;
    journey.until_scheme("dark").await?;
    assert_eq!(journey.token("--omfp-surface").await?, "#1e1e2e");
    let config = std::fs::read_to_string(journey.config_file())?;
    assert!(config.contains("theme = \"catppuccin\""), "{config}");

    // Every palette in both schemes, as every window draws it, untouched by the contrast guard (UI-196).
    let mut connection = journey.connect().await?;
    for (theme, light, dark) in THEME_SURFACES {
        connection
            .call::<omfp_protocol::method::SettingsSet>(omfp_protocol::SetSettingsParams {
                ui: Some(omfp_protocol::UiSettings {
                    theme: Some(serde_json::from_value(json!(theme))?),
                    ..omfp_protocol::UiSettings::default()
                }),
                keymap: None,
            })
            .await?;
        until_theme(&journey, theme).await?;
        for (scheme, surface) in [("light", light), ("dark", dark)] {
            journey.scheme(scheme).await?;
            assert_eq!(
                journey.token("--omfp-surface").await?,
                surface,
                "{theme} {scheme}"
            );
            assert_eq!(
                journey.attr("html", "data-contrast-guard").await?,
                None,
                "{theme} {scheme}: the guard overrides nothing"
            );
            journey
                .screenshot(&format!("theme-{theme}-{scheme}"))
                .await?;
        }
    }

    // The system accent comes from the macOS window only: a browser falls back to Synth.
    journey.command("theme accent").await?;
    journey
        .until("the accent setting", async |page| {
            let config = std::fs::read_to_string(page.config_file())?;
            Ok(config.contains("theme = \"accent\"").then_some(()))
        })
        .await?;
    until_theme(&journey, "synth").await?;

    // A user theme still wins over a built-in one; clearing it shows the built-in theme again.
    journey.command("theme gruvbox light").await?;
    until_theme(&journey, "gruvbox").await?;
    let gruvbox = journey.token("--omfp-primary").await?;
    connection
        .call::<omfp_protocol::method::ThemeSet>(omfp_protocol::Theme {
            css: Some(":root { --omfp-primary: #0b6e4f; }".to_owned()),
        })
        .await?;
    journey
        .until("the user theme's primary", async |page| {
            Ok((page.token("--omfp-primary").await? == "#0b6e4f").then_some(()))
        })
        .await?;
    connection
        .call::<omfp_protocol::method::ThemeSet>(omfp_protocol::Theme { css: None })
        .await?;
    journey
        .until("the built-in theme's primary", async |page| {
            Ok((page.token("--omfp-primary").await? == gruvbox).then_some(()))
        })
        .await?;
    connection.close().await?;
    journey.finish("built-in themes").await
}

/// Waits until `<html>` carries `data-density="<density>"`.
async fn until_density(journey: &Journey, density: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("the {density} density"), async |page| {
            let value = page.attr("html", "data-density").await?;
            Ok((value.as_deref() == Some(density)).then_some(()))
        })
        .await
}

/// Waits until `<html>`'s computed `font-size` is `size` (in px, e.g. `"17px"`).
async fn until_root_font_size(journey: &Journey, size: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("a root font size of {size}"), async |page| {
            let value = page
                .eval(
                    "return getComputedStyle(document.documentElement).fontSize;",
                    Vec::new(),
                )
                .await?;
            Ok((value.as_str() == Some(size)).then_some(()))
        })
        .await
}

/// Waits until `<html>` carries `data-motion="<motion>"`.
async fn until_motion(journey: &Journey, motion: &str) -> anyhow::Result<()> {
    journey
        .until(&format!("motion {motion}"), async |page| {
            let value = page.attr("html", "data-motion").await?;
            Ok((value.as_deref() == Some(motion)).then_some(()))
        })
        .await
}

// spec: DMN-009, UI-097, UI-141, UI-144, UI-145
#[tokio::test(flavor = "multi_thread")]
async fn editing_the_config_file_changes_the_open_page_live() -> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    until_density(&journey, "compact").await?;
    until_root_font_size(&journey, "17px").await?;
    until_motion(&journey, "full").await?;
    // The chrome is set in the bundled Commit Mono, served by the daemon with the bundle.
    journey
        .until("the bundled code face", async |page| {
            let loaded = page
                .eval(
                    "return [...document.fonts].some((f) => \
                     f.family.replaceAll('\"', '') === 'Commit Mono' && f.status === 'loaded');",
                    Vec::new(),
                )
                .await?;
            Ok((loaded == json!(true)).then_some(()))
        })
        .await?;
    let compact = journey.token("--omfp-row-h").await?;
    journey
        .eval("window.omfpE2eMarker = 'kept';", Vec::new())
        .await?;

    // The user edits the daemon's config file while the page is open.
    let config = journey.config_file();
    std::fs::write(
        &config,
        "# edited by hand\n[ui]\ndensity = \"normal\"\nfont_size = 20\nmotion = \"none\"\n",
    )?;
    until_density(&journey, "normal").await?;
    until_root_font_size(&journey, "20px").await?;
    until_motion(&journey, "none").await?;
    let normal = journey.token("--omfp-row-h").await?;
    assert_ne!(normal, compact, "the density changes the row height");
    let marker = journey
        .eval("return window.omfpE2eMarker || null;", Vec::new())
        .await?;
    assert_eq!(marker, json!("kept"), "the page did not reload");

    // And back.
    std::fs::write(&config, "[ui]\ndensity = \"compact\"\n")?;
    until_density(&journey, "compact").await?;
    until_root_font_size(&journey, "17px").await?;
    until_motion(&journey, "full").await?;
    assert_eq!(journey.token("--omfp-row-h").await?, compact);
    journey.finish("config file").await
}

/// `scrollTop` and whether the transcript is scrolled to its end, with its thread and pinned flag.
async fn transcript_scroll(journey: &Journey) -> anyhow::Result<(f64, bool, String, String)> {
    let value = journey
        .eval(
            "const t = document.querySelector('[data-testid=transcript]');
             return [t.scrollTop, t.scrollTop + t.clientHeight >= t.scrollHeight - 2,
                     t.dataset.threadId || '', t.dataset.pinned || ''];",
            Vec::new(),
        )
        .await?;
    Ok(serde_json::from_value(value)?)
}

/// A 1×1 PNG.
const DOT_PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

/// Pastes [`DOT_PNG`] into the composer.
async fn paste_image(journey: &Journey) -> anyhow::Result<()> {
    journey
        .eval(
            "const bytes = Uint8Array.from(atob(arguments[0]), c => c.charCodeAt(0));
             const data = new DataTransfer();
             data.items.add(new File([bytes], 'dot.png', { type: 'image/png' }));
             const input = document.querySelector('[data-testid=composer-input]');
             input.focus();
             input.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));",
            vec![json!(DOT_PNG)],
        )
        .await?;
    Ok(())
}

fn long_text() -> String {
    (1..=200).fold(String::new(), |mut text, line| {
        text.push_str("line ");
        text.push_str(&line.to_string());
        text.push_str("\n\n");
        text
    })
}

// spec: UI-042, UI-060, UI-064
#[tokio::test(flavor = "multi_thread")]
async fn the_transcript_keeps_its_scroll_position_per_thread_and_pasted_images_attach()
-> anyhow::Result<()> {
    let journey = Journey::start(vec![
        Step::reply(Reply::text(long_text())),
        Step::reply(Reply::text("Short one.")),
    ])
    .await?;
    journey.connected().await?;
    let long_thread = journey.create_thread("long thread").await?;
    journey.send("talk a lot").await?;
    journey.reply("line 200").await?;
    // Pinned to the end while the reply streams in.
    let (_, at_end, shown, pinned) = journey
        .until("the transcript at its end", async |page| {
            let scroll = transcript_scroll(page).await?;
            Ok(scroll.1.then_some(scroll))
        })
        .await?;
    assert_eq!(
        (at_end, shown.as_str(), pinned.as_str()),
        (true, long_thread.as_str(), "true")
    );

    // Scrolling up with the wheel unpins.
    journey
        .eval(
            "const t = document.querySelector('[data-testid=transcript]');
             t.dispatchEvent(new WheelEvent('wheel', { deltaY: -400, bubbles: true }));
             t.scrollTop = 400;",
            Vec::new(),
        )
        .await?;
    let (scrolled, ..) = journey
        .until("the transcript to unpin", async |page| {
            let scroll = transcript_scroll(page).await?;
            Ok((scroll.3 == "false").then_some(scroll))
        })
        .await?;

    // Another thread starts at its own end; its draft is its own (UI-064).
    journey
        .type_into("[data-testid=composer-input]", "draft of the long thread")
        .await?;
    let short_thread = journey.create_thread("short thread").await?;
    journey.send("briefly").await?;
    journey.reply("Short one.").await?;
    let (_, at_end, shown, pinned) = transcript_scroll(&journey).await?;
    assert_eq!(
        (at_end, shown.as_str(), pinned.as_str()),
        (true, short_thread.as_str(), "true")
    );

    // A pasted image becomes an attachment (UI-060).
    paste_image(&journey).await?;
    journey
        .until("the pasted image", async |page| {
            Ok((page.count("[data-testid=attachment]").await? == 1).then_some(()))
        })
        .await?;

    // Back on the long thread: where the user left it, unpinned, with its draft.
    journey.click(&row(&long_thread)).await?;
    journey.opened(&long_thread).await?;
    let (top, _, shown, pinned) = journey
        .until("the long thread's scroll position", async |page| {
            let scroll = transcript_scroll(page).await?;
            Ok((scroll.2 == long_thread && (scroll.0 - scrolled).abs() < 2.0).then_some(scroll))
        })
        .await?;
    assert_eq!(
        (shown.as_str(), pinned.as_str()),
        (long_thread.as_str(), "false"),
        "{top}"
    );
    assert_eq!(draft(&journey).await?, "draft of the long thread");
    assert_eq!(journey.count("[data-testid=attachment]").await?, 0);
    journey.finish("browser behaviour").await
}

/// Waits until the sidebar shows the thread `thread_id` `count` times.
async fn until_rows(journey: &Journey, thread_id: &str, count: usize) -> anyhow::Result<()> {
    let css = row(thread_id);
    journey
        .until(&format!("{count} rows of {thread_id}"), async |page| {
            Ok((page.count(&css).await? == count).then_some(()))
        })
        .await
}

// spec: UI-020, UI-026, UI-482, DMN-028
#[tokio::test(flavor = "multi_thread")]
async fn an_archived_thread_is_found_under_archived_unarchived_then_deleted_with_workspaces()
-> anyhow::Result<()> {
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    let survivor = journey.create_thread("survivor").await?;
    let doomed = journey.create_thread("doomed").await?;

    // Archiving (from the header's ⋯, UI-482) hides it and leaves no thread selected.
    journey.click("[data-testid=header-more]").await?;
    journey.click("[data-testid=archive-thread]").await?;
    until_rows(&journey, &doomed, 0).await?;
    journey.element("[data-testid=no-thread]").await?;

    // "Archived (1)" at the end of its workspace lists it; it opens from there and comes back with Unarchive.
    journey.click("[data-testid=archived-toggle]").await?;
    until_rows(&journey, &doomed, 1).await?;
    journey.click(&row(&doomed)).await?;
    journey.opened(&doomed).await?;
    journey.click("[data-testid=header-more]").await?;
    journey.click("[data-testid=archive-thread]").await?;
    journey.gone("[data-testid=archived-toggle]").await?;
    until_rows(&journey, &doomed, 1).await?;

    // The row's delete button (shown on hover) deletes immediately; the workspace stays.
    let doomed_row = row(&doomed);
    journey.hover(&doomed_row).await?;
    journey
        .click(&format!(
            "[data-testid=row-delete-thread][data-thread-id=\"{doomed}\"]"
        ))
        .await?;
    until_rows(&journey, &doomed, 0).await?;
    journey
        .until("the thread directory to go", async |page| {
            Ok((!page.data_dir().join("threads").join(&doomed).exists()).then_some(()))
        })
        .await?;
    assert_eq!(journey.count("[data-testid=workspace-section]").await?, 2);

    // Deleting a workspace that still has a thread lists it and deletes it too once asked to (DMN-028).
    let section = "[data-testid=workspace-section]:has([data-testid=thread-row])";
    journey
        .hover(&format!("{section} [data-testid=workspace-section-name]"))
        .await?;
    journey
        .click(&format!("{section} [data-testid=delete-workspace]"))
        .await?;
    journey
        .element(&format!(
            "[data-testid=delete-workspace-thread-list] [data-thread-id=\"{survivor}\"]"
        ))
        .await?;
    journey
        .click("[data-testid=delete-workspace-threads]")
        .await?;
    journey
        .click("[data-testid=confirm-delete-workspace]")
        .await?;
    until_rows(&journey, &survivor, 0).await?;
    journey
        .until("one workspace left", async |page| {
            Ok((page.count("[data-testid=workspace-section]").await? == 1).then_some(()))
        })
        .await?;

    // Gone after a reload too.
    journey.driver.refresh().await?;
    journey.connected().await?;
    journey.element("[data-testid=workspace-section]").await?;
    assert_eq!(journey.count("[data-testid=workspace-section]").await?, 1);
    assert_eq!(journey.count("[data-testid=thread-row]").await?, 0);
    journey.finish("archive and delete").await
}

// spec: UI-025, DMN-021
#[tokio::test(flavor = "multi_thread")]
async fn a_repository_seen_in_one_workspace_is_suggested_in_another() -> anyhow::Result<()> {
    let repo = repository("seen-3");
    let journey = Journey::start(Vec::new()).await?;
    journey.connected().await?;
    journey.create_thread("first sighting").await?;
    link(&journey, &repo.path().to_string_lossy()).await?;

    // In another workspace the empty path field suggests it first; picking it fills the field, Add links it.
    journey.create_thread("second sighting").await?;
    journey
        .until("the second workspace in the widget", async |page| {
            let name = page.text("[data-testid=workspace-name]").await?;
            let empty = page.count("[data-testid=workspace-repo]").await? == 0;
            Ok((name.contains("second sighting") && empty).then_some(()))
        })
        .await?;
    journey.click("[data-testid=add-repo-path]").await?;
    let suggestion = format!(
        "[data-testid=path-option][data-kind=suggestion][data-path={}]",
        json!(repo.path().canonicalize()?.to_string_lossy())
    );
    journey.click(&suggestion).await?;
    journey.click("[data-testid=add-repo-submit]").await?;
    let name = new_entry(
        &journey,
        "[data-testid=workspace-repo][data-kind=link]",
        &[],
    )
    .await?;
    assert_eq!(name, dir_name(&repo));
    journey.finish("suggestions").await
}

/// The computed text color of the first highlighted keyword of the reply's code block.
async fn keyword_color(journey: &Journey) -> anyhow::Result<String> {
    let value = journey
        .eval(
            "const k = document.querySelector('[data-testid=markdown] pre .hljs-keyword');
             return k === null ? null : getComputedStyle(k).color;",
            Vec::new(),
        )
        .await?;
    Ok(value.as_str().unwrap_or_default().to_owned())
}

/// The computed color of `--omfp-primary` (as `rgb(…)`), the color keywords take.
async fn primary_color(journey: &Journey) -> anyhow::Result<String> {
    let value = journey
        .eval(
            "const probe = document.createElement('span');
             probe.style.color = 'var(--omfp-primary)';
             document.body.append(probe);
             const color = getComputedStyle(probe).color;
             probe.remove();
             return color;",
            Vec::new(),
        )
        .await?;
    Ok(value.as_str().unwrap_or_default().to_owned())
}

/// Waits until the reply's keywords have the theme's primary color.
async fn keywords_follow_the_theme(journey: &Journey) -> anyhow::Result<String> {
    journey
        .until("keywords in the theme's primary color", async |page| {
            let keyword = keyword_color(page).await?;
            Ok((!keyword.is_empty() && keyword == primary_color(page).await?).then_some(keyword))
        })
        .await
}

/// The reply's code block is highlighted with the theme's colors: light, dark and a custom theme (UI-090).
async fn code_follows_the_theme(journey: &Journey) -> anyhow::Result<()> {
    journey.scheme("light").await?;
    let light = keywords_follow_the_theme(journey).await?;
    journey.screenshot("m5-code-light").await?;
    journey.scheme("dark").await?;
    let dark = keywords_follow_the_theme(journey).await?;
    assert_ne!(light, dark, "keywords change color with the scheme");
    journey.screenshot("m5-code-dark").await?;
    // A color that reaches AA on the dark surfaces, so the contrast guard (UI-196) keeps it as given.
    let mut connection = journey.connect().await?;
    connection
        .call::<omfp_protocol::method::ThemeSet>(omfp_protocol::Theme {
            css: Some(":root { --omfp-primary: #ff80ab; }\n".to_owned()),
        })
        .await?;
    connection.close().await?;
    journey
        .until("the custom theme's keywords", async |page| {
            Ok((keyword_color(page).await? == "rgb(255, 128, 171)").then_some(()))
        })
        .await?;
    journey.screenshot("m5-code-custom").await?;
    Ok(())
}

// spec: E2E-06, UI-080, UI-090, DMN-026, UI-025, DMN-023
#[tokio::test(flavor = "multi_thread")]
async fn a_file_the_agent_edits_shows_its_diff_in_the_changes_widget() -> anyhow::Result<()> {
    let repo = repository("fig-6");
    let name = dir_name(&repo);
    let file = format!("{name}/AGENTS.md");
    let call = |id: &str, tool: &str, args: serde_json::Value| {
        Step::reply(Reply::tool_calls(vec![ToolCall::new(id, tool, args)]))
    };
    let journey = Journey::start(vec![
        call("call_read_1", "read", json!({ "path": file })),
        call(
            "call_write_1",
            "write",
            json!({ "path": file, "content": "Marker: fig-6\nEdited by the agent.\n" }),
        ),
        call("call_grep_1", "grep", json!({ "pattern": "Edited", "path": name })),
        call(
            "call_todo_1",
            "todo",
            json!({ "op": "init", "items": ["Edit AGENTS.md", "Show the diff"] }),
        ),
        // Open items make omp remind the model to go on: the journey finishes them.
        call(
            "call_todo_2",
            "todo",
            json!({ "op": "done", "task": "Edit AGENTS.md" }),
        ),
        call(
            "call_todo_3",
            "todo",
            json!({ "op": "done", "task": "Show the diff" }),
        ),
        call(
            "call_bash_1",
            "bash",
            json!({ "command": format!("git -C {name} status --short") }),
        ),
        Step::reply(Reply::text(
            "Edited `AGENTS.md`:\n\n```rust\nfn main() {\n    let edited = true;\n    println!(\"{edited}\");\n}\n```\n",
        )),
    ])
    .await?;
    journey.connected().await?;
    journey
        .new_linked_workspace(&partial(repo.path()), &repo.path().to_string_lossy())
        .await?;
    let thread = journey.first_message("edit the agents file").await?;
    journey.reply("Edited").await?;
    journey.status(&thread, "idle").await?;
    for tool in ["read", "write", "grep", "todo", "bash"] {
        journey
            .element(&format!(
                "[data-testid=tool-card][data-tool={tool}][data-status=succeeded]"
            ))
            .await?;
    }
    // The workspace widget shows the edited repository as dirty, without a refresh (UI-025, DMN-023).
    let edited = format!(
        "[data-widget=\"omfp:workspace\"] [data-testid=workspace-repo][data-name=\"{name}\"][data-dirty=true]"
    );
    journey.element(&edited).await?;
    assert_eq!(
        journey
            .text(&format!("{edited} [data-testid=git-status]"))
            .await?,
        "dirty · 1 file +1 −0"
    );
    journey.screenshot("m5-transcript").await?;

    changes_widget_shows_the_diff(&journey).await?;
    code_follows_the_theme(&journey).await?;
    journey.finish("E2E-06").await
}

/// The Changes tab next to the workspace widget lists the edited file; it expands to its diff.
async fn changes_widget_shows_the_diff(journey: &Journey) -> anyhow::Result<()> {
    let clicked = journey
        .eval(
            "const tab = [...document.querySelectorAll('[data-testid=view-tab]')]
               .find(t => t.textContent === 'Changes');
             if (tab) { tab.click(); }
             return tab !== undefined;",
            Vec::new(),
        )
        .await?;
    assert_eq!(clicked, json!(true), "the default view has a Changes tab");
    let entry = "[data-testid=changes-file][data-path=\"AGENTS.md\"]";
    journey.element(entry).await?;
    // The counts count up from 0 at motion `full` (UI-146): wait for their final value. They are two flex items, so
    // `innerText` puts a line break between them.
    let counts = format!("{entry} [data-testid=changes-counts]");
    journey
        .until("the file's +/− counts", async |page| {
            let text = page.texts(&counts).await?.into_iter().next();
            let words = text.map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "));
            Ok((words.as_deref() == Some("+1 −0")).then_some(()))
        })
        .await?;
    journey
        .click(&format!("{entry} [data-testid=changes-file-toggle]"))
        .await?;
    // The diff is the CodeMirror merge view of the whole file (UI-080).
    let added = journey
        .until("the diff's added line", async |page| {
            let rows = page
                .texts("[data-testid=changes-diff][data-view=merge] .cm-content .cm-changedLine")
                .await?;
            Ok((!rows.is_empty()).then_some(rows))
        })
        .await?;
    assert!(
        added.iter().any(|row| row.contains("Edited by the agent.")),
        "{added:?}"
    );
    journey.screenshot("m5-changes").await?;
    Ok(())
}

// spec: E2E-21, UI-270, UI-271, UI-274, UI-373, PRT-100
#[tokio::test(flavor = "multi_thread")]
async fn a_background_job_runs_on_its_card_the_badge_and_the_background_tab() -> anyhow::Result<()>
{
    // The job prints a line, then waits for the test to release it.
    let command =
        "echo bg-e2e-started; while [ ! -f release.flag ]; do sleep 0.1; done; echo bg-e2e-done";
    let journey = Journey::start(vec![
        Step::reply(Reply::tool_calls(vec![ToolCall::new(
            "call_bash_bg",
            "bash",
            json!({ "command": command, "async": true }),
        )])),
        Step::reply(Reply::text("Started it in the background.")),
        Step::reply(Reply::text("The job is done.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("background journey").await?;
    let folder = journey.workspace_folder(&thread).await?;
    journey.send("run it in the background").await?;

    // The call ended, the job runs: a running card marked background with its last line, and the header's badge.
    journey
        .element("[data-testid=tool-card][data-status=running] [data-testid=tool-background][data-state=running]")
        .await?;
    journey
        .until("the job's last line", async |page| {
            let line = page.text("[data-testid=tool-last-line]").await?;
            Ok((line == "bg-e2e-started").then_some(()))
        })
        .await?;
    journey.reply("Started it in the background.").await?;
    journey
        .until("the badge", async |page| {
            let badge = page.text("[data-testid=background-badge]").await?;
            Ok((badge == "1 in background").then_some(()))
        })
        .await?;

    // The badge shows the default view's Background tab with the job.
    journey.click("[data-testid=background-badge]").await?;
    journey
        .element("[data-testid=background-row][data-kind=job][data-status=running]")
        .await?;
    journey.screenshot("m12-background").await?;

    // A view without Background but with the `main` slot: the badge opens Background as a tab there (UI-373).
    let manifest = json!({
        "default": "lean",
        "views": {
            "lean": { "layout": ["col", {}, "omfp:header", ["tabs", { "id": "main" }, "omfp:chat"]] }
        }
    });
    std::fs::write(
        folder.join(".omfp/views.json"),
        serde_json::to_string_pretty(&manifest)?,
    )?;
    journey
        .element("[data-testid=view][data-view=lean]")
        .await?;
    journey.click("[data-testid=background-badge]").await?;
    let tab = "[data-slot=main] [data-testid=widget-tab][data-tab=\"widget:omfp:background\"]";
    journey
        .element(&format!("{tab}[aria-selected=true]"))
        .await?;
    assert_eq!(journey.text(tab).await?, "Background");
    journey
        .element(
            "[data-slot=main] [data-testid=background-row][data-kind=job][data-status=running]",
        )
        .await?;
    assert_eq!(journey.count("[data-testid=floating-widget]").await?, 0);
    journey.screenshot("background-main-tab").await?;

    // Released, the job completes: the card and the row settle, the badge goes.
    std::fs::write(folder.join("release.flag"), "")?;
    journey
        .element(
            "[data-slot=main] [data-testid=background-row][data-kind=job][data-status=completed]",
        )
        .await?;
    journey.gone("[data-testid=background-badge]").await?;
    // The tab closes by its own ×.
    journey
        .click(&format!("{tab} [data-testid=widget-tab-close]"))
        .await?;
    journey.gone(tab).await?;
    one_tool_card(&journey, "succeeded").await?;
    journey.reply("The job is done.").await?;
    journey.status(&thread, "idle").await?;
    journey.finish("E2E-21").await
}

/// Pastes `text` into the composer as the browser does, with `text/plain` on the clipboard.
async fn paste_text(journey: &Journey, text: &str) -> anyhow::Result<()> {
    journey
        .eval(
            "const data = new DataTransfer();
             data.setData('text/plain', arguments[0]);
             const input = document.querySelector('[data-testid=composer-input]');
             input.focus();
             input.setSelectionRange(input.value.length, input.value.length);
             input.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));",
            vec![json!(text)],
        )
        .await?;
    Ok(())
}

// spec: E2E-29, UI-357, UI-360
#[tokio::test(flavor = "multi_thread")]
async fn a_long_paste_is_a_chip_in_the_composer_and_the_agent_gets_the_whole_text()
-> anyhow::Result<()> {
    let pasted = (1..=40).fold(String::new(), |mut text, line| {
        use std::fmt::Write as _;
        writeln!(text, "pasted row {line}").expect("writing to a String");
        text
    });
    let journey = Journey::start(vec![Step::when(
        fake_llm::Matcher::LastUserContains("Summarize: pasted row 1\n".to_owned()),
        Reply::text("Forty rows, all read."),
    )])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("paste thread").await?;
    journey
        .type_into("[data-testid=composer-input]", "Summarize: ")
        .await?;
    paste_text(&journey, &pasted).await?;
    let chips = journey
        .until("the paste chip", async |page| {
            let chips = page.texts("[data-testid=paste-toggle]").await?;
            Ok((!chips.is_empty()).then_some(chips))
        })
        .await?;
    assert_eq!(chips, ["Pasted text #1 · 41 lines"]);
    assert_eq!(draft(&journey).await?, "Summarize: [Paste #1, +41 lines]");

    journey.click("[data-testid=send]").await?;
    journey.reply("Forty rows, all read.").await?;
    let users = journey
        .texts("[data-testid=message][data-role=user]")
        .await?;
    assert!(
        users
            .iter()
            .any(|text| text.contains("pasted row 1") && text.contains("pasted row 40")),
        "{users:?}"
    );
    assert!(
        !users.iter().any(|text| text.contains("[Paste #1")),
        "{users:?}"
    );
    assert_eq!(journey.count("[data-testid=paste-chip]").await?, 0);
    journey.status(&thread, "idle").await?;
    journey.finish("E2E-29").await
}

// spec: E2E-30, UI-361, UI-362, DMN-216, DMN-218, PRT-127
#[tokio::test(flavor = "multi_thread")]
async fn up_recalls_the_sent_prompts_newest_first_and_down_restores_the_draft() -> anyhow::Result<()>
{
    let journey = Journey::start(vec![
        Step::reply(Reply::text("First answer.")),
        Step::reply(Reply::text("Second answer.")),
    ])
    .await?;
    journey.connected().await?;
    let thread = journey.create_thread("history thread").await?;
    journey.send("first prompt").await?;
    journey.reply("First answer.").await?;
    journey.status(&thread, "idle").await?;
    journey.send("second prompt").await?;
    journey.reply("Second answer.").await?;
    journey.status(&thread, "idle").await?;

    journey
        .type_into("[data-testid=composer-input]", "half typed")
        .await?;
    for expected in ["second prompt", "first prompt"] {
        journey.keys(Key::Up).await?;
        journey
            .until(
                &format!("the composer to show {expected:?}"),
                async |page| Ok((draft(page).await? == expected).then_some(())),
            )
            .await?;
    }
    journey.keys(Key::Down).await?;
    assert_eq!(draft(&journey).await?, "second prompt");
    journey.keys(Key::Down).await?;
    assert_eq!(draft(&journey).await?, "half typed");
    journey.finish("E2E-30").await
}
