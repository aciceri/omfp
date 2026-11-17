import { afterEach, describe, expect, mock, test } from "bun:test";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import {
  BorderBox,
  BoxRule,
  Button,
  Card,
  ChatHeader,
  ChatTitle,
  Checkbox,
  Code,
  CommandItem,
  ComposerBar,
  ComposerHint,
  ConfirmDialog,
  compactNumber,
  Diff,
  DiffStat,
  ExitCode,
  HelpDialog,
  HelpRow,
  HelpSection,
  InteractionCard,
  Log,
  Markdown,
  Menu,
  MenuItem,
  Message,
  Meter,
  ModeLine,
  ModeNotice,
  ModeSegment,
  ModeViews,
  motionLevel,
  NestedRow,
  PathField,
  QueueChip,
  QueueList,
  ResolvedLine,
  RowList,
  Select,
  StateBadge,
  StateMark,
  StreamCaret,
  Tab,
  TabList,
  Tabs,
  TextField,
  Thinking,
  ThreadRow,
  Toast,
  TodoList,
  ToggleRow,
  ToolCard,
  WorkspaceSection,
} from "../src/index.ts";

afterEach(() => {
  cleanup();
  delete document.documentElement.dataset.motion;
});

describe("Button", () => {
  // spec: UI-149
  test("shows a leading icon and a trailing key chip outside its accessible name", () => {
    render(
      <Button variant="primary" icon={<svg data-testid="icon" />} kbd="enter">
        Send
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Send" });
    expect(button.getAttribute("data-variant")).toBe("primary");
    expect(button.querySelector("[data-testid=icon]")).not.toBeNull();
    const chip = button.querySelector("kbd");
    expect(chip?.textContent).toBe("enter");
    expect(chip?.getAttribute("aria-hidden")).toBe("true");
  });

  // spec: UI-149, UI-133
  test("busy keeps the label, spins an icon and ignores presses", async () => {
    const user = userEvent.setup();
    const onPress = mock(() => undefined);
    render(
      <Button isPending onPress={onPress} data-testid="go">
        Remove
      </Button>,
    );
    const button = screen.getByTestId("go");
    expect(button.textContent).toBe("Remove");
    expect(button.hasAttribute("data-pending")).toBe(true);
    expect(button.querySelector(".omfp-button__spinner")).not.toBeNull();
    await user.click(button);
    expect(onPress).not.toHaveBeenCalled();
  });
});

describe("TextField", () => {
  // spec: UI-150
  test("a box with icon, key chip and mono text; invalid shows an error line with an alert icon", () => {
    render(
      <TextField
        label="Path"
        icon={<svg data-testid="icon" />}
        kbd="tab"
        mono
        errorMessage="No such folder"
        defaultValue="~/projects"
        data-testid="field"
      />,
    );
    const field = screen.getByTestId("field");
    const box = field.querySelector(".omfp-input-box");
    expect(box?.getAttribute("data-mono")).toBe("true");
    expect(box?.querySelector("[data-testid=icon]")).not.toBeNull();
    expect(box?.querySelector("kbd")?.textContent).toBe("tab");
    expect(field.hasAttribute("data-invalid")).toBe(true);
    const error = field.querySelector(".omfp-field__error");
    expect(error?.textContent).toBe("No such folder");
    expect(error?.querySelector("[data-icon=circle-alert]")).not.toBeNull();
    expect(screen.getByRole("textbox", { name: "Path" })).toBeDefined();
  });
});

describe("Select", () => {
  // spec: UI-152
  test("shows its prefix, value and chevron; the open list checks the selected option and shows metadata", async () => {
    const user = userEvent.setup();
    render(
      <Select
        label="Thinking"
        hideLabel
        prefix="thinking"
        items={[
          { id: "low", label: "low" },
          { id: "high", label: "high", meta: "slow" },
        ]}
        selectedKey="high"
        onSelectionChange={() => undefined}
        data-testid="select"
      />,
    );
    const select = screen.getByTestId("select");
    expect(select.querySelector(".omfp-select__prefix")?.textContent).toBe("thinking");
    expect(select.querySelector(".omfp-select__chevron")).not.toBeNull();
    await user.click(screen.getByRole("button"));
    const options = await screen.findAllByRole("option");
    expect(options.map((option) => option.querySelector(".omfp-option__check svg") !== null)).toEqual([false, true]);
    expect(options[1]?.querySelector(".omfp-option__meta")?.textContent).toBe("slow");
  });
});

describe("Checkbox", () => {
  // spec: UI-153
  test("checked and mixed states are exposed for the box's check and dash", () => {
    render(
      <>
        <Checkbox defaultSelected data-testid="checked">
          Checked
        </Checkbox>
        <Checkbox isIndeterminate data-testid="mixed">
          Mixed
        </Checkbox>
      </>,
    );
    expect(screen.getByTestId("checked").hasAttribute("data-selected")).toBe(true);
    expect(screen.getByTestId("checked").querySelector(".omfp-checkbox__check")).not.toBeNull();
    expect(screen.getByTestId("mixed").hasAttribute("data-indeterminate")).toBe(true);
    expect(screen.getByTestId("mixed").querySelector(".omfp-checkbox__dash")).not.toBeNull();
  });
});

describe("Tabs", () => {
  // spec: UI-154
  test("the selected tab is marked for its glyph and indicator", () => {
    render(
      <Tabs defaultSelectedKey="changes">
        <TabList aria-label="Panels">
          <Tab id="workspace">Workspace</Tab>
          <Tab id="changes">Changes</Tab>
        </TabList>
      </Tabs>,
    );
    expect(screen.getByRole("tab", { name: "Changes" }).hasAttribute("data-selected")).toBe(true);
    expect(screen.getByRole("tab", { name: "Workspace" }).hasAttribute("data-selected")).toBe(false);
  });
});

describe("Menu", () => {
  // spec: UI-155
  test("items show an icon, a label and a key chip; destructive items are marked", () => {
    render(
      <Menu aria-label="Workspace">
        <MenuItem id="rename" icon={<svg data-testid="icon" />} kbd="r">
          Rename
        </MenuItem>
        <MenuItem id="remove" danger kbd="d d">
          Remove workspace
        </MenuItem>
      </Menu>,
    );
    const rename = screen.getByRole("menuitem", { name: "Rename" });
    expect(rename.querySelector(".omfp-menu__label")?.textContent).toBe("Rename");
    expect(rename.querySelector("kbd")?.textContent).toBe("r");
    expect(screen.getByRole("menuitem", { name: "Remove workspace" }).getAttribute("data-danger")).toBe("true");
  });
});

describe("ConfirmDialog", () => {
  // spec: UI-156
  test("confirming: the confirm button is busy and the dialog stays; an error shows above the actions", async () => {
    const user = userEvent.setup();
    const onConfirm = mock(() => undefined);
    render(
      <ConfirmDialog
        isOpen
        onOpenChange={() => undefined}
        title="Remove?"
        message="Its folder is deleted."
        confirmLabel="Remove"
        tone="danger"
        cancelKbd="esc"
        confirmKbd="enter"
        busy
        error="The folder is busy"
        onConfirm={onConfirm}
      />,
    );
    const confirm = screen.getByTestId("confirm");
    expect(confirm.hasAttribute("data-pending")).toBe(true);
    expect(screen.getByTestId("cancel").querySelector("kbd")?.textContent).toBe("esc");
    expect(screen.getByRole("alert").textContent).toBe("The folder is busy");
    await user.click(confirm);
    expect(onConfirm).not.toHaveBeenCalled();
    expect(screen.getByTestId("confirm-dialog")).toBeDefined();
  });

  // spec: UI-156, UI-173
  test("renders in place without the modal overlay", () => {
    render(
      <ConfirmDialog
        inline
        isOpen
        onOpenChange={() => undefined}
        title="Remove?"
        message="Gone."
        confirmLabel="Remove"
        onConfirm={() => undefined}
      />,
    );
    expect(document.querySelector(".omfp-modal-overlay")).toBeNull();
    expect(screen.getByRole("alertdialog", { name: "Remove?" })).toBeDefined();
  });
});

describe("marks", () => {
  // spec: UI-122, UI-137, UI-157
  test("state marks and badges carry their state; decorative unless labelled", () => {
    render(
      <>
        <StateMark state="running" data-testid="mark" />
        <StateMark state="failed" label="Failed" />
        <StateBadge state="awaiting" fresh data-testid="badge" />
      </>,
    );
    expect(screen.getByTestId("mark").getAttribute("data-state")).toBe("running");
    expect(screen.getByTestId("mark").getAttribute("aria-hidden")).toBe("true");
    expect(screen.getByRole("img", { name: "Failed" }).getAttribute("data-state")).toBe("failed");
    const badge = screen.getByTestId("badge");
    expect(badge.textContent).toBe("awaiting");
    expect(badge.getAttribute("data-fresh")).toBe("true");
  });

  // spec: UI-483
  test("a bare state badge says so (no box); a badge is boxed by default", () => {
    render(
      <>
        <StateBadge state="running" bare data-testid="bare" />
        <StateBadge state="running" data-testid="boxed" />
      </>,
    );
    expect([screen.getByTestId("bare").dataset.bare, screen.getByTestId("boxed").dataset.bare]).toEqual([
      "true",
      undefined,
    ]);
  });

  // spec: UI-137, UI-157
  test("exit codes: 0 is ok, anything else is not", () => {
    render(
      <>
        <ExitCode code={0} data-testid="ok" />
        <ExitCode code={2} data-testid="err" />
      </>,
    );
    expect(screen.getByTestId("ok").textContent).toBe("exit 0");
    expect(screen.getByTestId("ok").getAttribute("data-ok")).toBe("true");
    expect(screen.getByTestId("err").getAttribute("data-ok")).toBe("false");
  });

  // spec: UI-158
  test("token counts are compact", () => {
    expect([0, 999, 1000, 48_200, 171_000, 200_000, 1_500_000].map(compactNumber)).toEqual([
      "0",
      "999",
      "1k",
      "48.2k",
      "171k",
      "200k",
      "1.5M",
    ]);
  });

  // spec: UI-158
  test("the meter shows used/total and turns high above its threshold", () => {
    render(
      <>
        <Meter value={48_200} max={200_000} label="Context" data-testid="low" />
        <Meter value={171_000} max={200_000} label="Context" data-testid="high" />
      </>,
    );
    expect(screen.getByTestId("low").textContent).toBe("48.2k/200k");
    expect(screen.getByTestId("low").hasAttribute("data-high")).toBe(false);
    expect(screen.getByTestId("high").getAttribute("data-high")).toBe("true");
    expect(screen.getAllByRole("meter")).toHaveLength(2);
  });

  // spec: UI-165, UI-145, UI-446
  test("diff stats show their final numbers at once unless motion is full, a space apart", () => {
    document.documentElement.dataset.motion = "reduced";
    render(<DiffStat added={42} removed={7} data-testid="stat" />);
    expect(screen.getByTestId("stat").textContent).toBe("+42 −7");
  });

  // spec: UI-165, UI-146
  test("at motion full diff stats count up from 0", async () => {
    document.documentElement.dataset.motion = "full";
    render(<DiffStat added={42} removed={7} data-testid="stat" />);
    expect(screen.getByTestId("stat").textContent).toBe("+0 −0");
    await waitFor(() => expect(screen.getByTestId("stat").textContent).toBe("+42 −7"), { timeout: 2000 });
  });

  // spec: UI-145
  test("the effective motion level is data-motion, else the system preference", () => {
    document.documentElement.dataset.motion = "none";
    expect(motionLevel()).toBe("none");
    delete document.documentElement.dataset.motion;
    expect(motionLevel()).toBe("full");
  });
});

describe("Log", () => {
  // spec: UI-159
  test("structured lines have a time and a level column; an empty log shows one muted line", () => {
    const { rerender } = render(<Log lines={[]} data-testid="log" />);
    expect(screen.getByTestId("log").textContent).toBe("no output");
    rerender(<Log lines={[{ time: "14:02:11", level: "warn", text: "slow test" }, "plain line"]} data-testid="log" />);
    const log = screen.getByTestId("log");
    expect(log.querySelector(".omfp-log__level")?.getAttribute("data-level")).toBe("warn");
    expect([...log.querySelectorAll(".omfp-log__line")].map((line) => line.textContent)).toEqual([
      "14:02:11 warn slow test",
      "plain line",
    ]);
  });
});

describe("Diff", () => {
  // spec: UI-165
  test("an empty diff says so, a binary file too", () => {
    const { rerender } = render(<Diff patch="" data-testid="diff" />);
    expect(screen.getByTestId("diff").textContent).toBe("no changes");
    rerender(<Diff patch="" binary data-testid="diff" />);
    expect(screen.getByTestId("diff").textContent).toBe("binary file");
  });
});

describe("Toast", () => {
  // spec: UI-160
  test("info is a status that dismisses itself; errors are alerts that stay", async () => {
    const dismissed = mock(() => undefined);
    const stays = mock(() => undefined);
    render(
      <>
        <Toast onDismiss={dismissed} duration={20}>
          Saved
        </Toast>
        <Toast tone="error" onDismiss={stays} duration={1} action={<Button>Retry</Button>}>
          Daemon unreachable
        </Toast>
      </>,
    );
    expect(screen.getByRole("status").textContent).toContain("Saved");
    expect(screen.getByRole("alert").textContent).toContain("Daemon unreachable");
    // The error's own deadline (1 ms) passed before the info's (20 ms): it would have fired by now.
    await waitFor(() => expect(dismissed).toHaveBeenCalledTimes(1));
    expect(stays).not.toHaveBeenCalled();
  });
});

function ToggledTool({ body }: { body: boolean }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <ToolCard
      status="done"
      name="bash"
      arg="ls -la"
      meta="1.2s"
      expanded={expanded}
      onExpandedChange={setExpanded}
      data-testid="tool"
    >
      {body ? <pre data-testid="output">total 0</pre> : undefined}
    </ToolCard>
  );
}

describe("ToolCard", () => {
  // spec: UI-163
  test("the head toggles the body", async () => {
    const user = userEvent.setup();
    render(<ToggledTool body />);
    const head = screen.getByRole("button", { expanded: false });
    expect(screen.queryByTestId("output")).toBeNull();
    await user.click(head);
    expect(head.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByTestId("output")).toBeDefined();
    expect(screen.getByTestId("tool").getAttribute("data-status")).toBe("done");
  });

  // spec: UI-163
  test("a head without a body is not clickable", () => {
    render(<ToggledTool body={false} />);
    expect(screen.queryByRole("button")).toBeNull();
    expect(screen.getByTestId("tool-head").textContent).toBe("bashls -la1.2s");
    expect(screen.getByTestId("tool").classList.contains("omfp-bbox")).toBe(false);
  });

  // spec: UI-163, UI-216
  test("a running tool card's mark uses omp's status spinner; a thread mark keeps the activity spinner", () => {
    render(
      <>
        <ToolCard status="running" name="bash" data-testid="tool" />
        <StateMark state="running" data-testid="mark" />
      </>,
    );
    const toolMark = screen.getByTestId("tool").querySelector(".omfp-state-mark");
    expect(toolMark?.getAttribute("data-spinner")).toBe("status");
    expect(screen.getByTestId("mark").hasAttribute("data-spinner")).toBe(false);
  });

  // spec: UI-163, UI-187
  test("while the body shows, the whole head sits on the top border; collapsed, the card is one row again", async () => {
    const user = userEvent.setup();
    render(<ToggledTool body />);
    const tool = screen.getByTestId("tool");
    const head = screen.getByRole("button", { expanded: false });
    expect(tool.classList.contains("omfp-bbox")).toBe(false);
    expect(head.querySelector(".omfp-bbox__line")).toBeNull();
    await user.click(head);
    expect(tool.classList.contains("omfp-bbox")).toBe(true);
    expect(head.classList.contains("omfp-bbox__head")).toBe(true);
    expect(head.querySelector(".omfp-bbox__label")?.textContent).toBe("bashls -la1.2s");
    expect(head.querySelector(".omfp-bbox__line")?.getAttribute("aria-hidden")).toBe("true");
    expect(tool.querySelector(".omfp-tool__body")?.classList.contains("omfp-bbox__body")).toBe(true);
    expect(screen.getByTestId("tool-head") === head).toBe(true);
    await user.click(head);
    expect(tool.classList.contains("omfp-bbox")).toBe(false);
  });
});

describe("TodoList", () => {
  // spec: UI-447, UI-163
  test("items carry omp's status and its word for screen readers", () => {
    render(
      <TodoList
        itemTestId="task"
        items={[
          { text: "Read the code", status: "completed", statusLabel: "done" },
          { text: "Fix the bug", status: "in_progress", statusLabel: "in progress" },
          { text: "Old idea", status: "abandoned", statusLabel: "abandoned" },
        ]}
      />,
    );
    expect(
      screen.getAllByTestId("task").map((task) => [task.dataset.status, task.textContent, task.childElementCount]),
    ).toEqual([
      ["completed", "●Read the codedone", 3],
      ["in_progress", "●Fix the bugin progress", 3],
      ["abandoned", "●Old ideaabandoned", 3],
    ]);
    const [, , abandoned] = screen.getAllByTestId("task");
    expect(abandoned?.querySelector(".omfp-visually-hidden")?.textContent).toBe("abandoned");
  });
});

describe("BorderBox", () => {
  // spec: UI-187
  test("the title is a heading on the top border, the end part after the line, the body under it", () => {
    render(
      <BorderBox title="Repositories" level={3} titleId="repos" end="0:42" aria-labelledby="repos" data-testid="box">
        <span data-testid="content">rows</span>
      </BorderBox>,
    );
    const box = screen.getByRole("region", { name: "Repositories" });
    expect(box === screen.getByTestId("box")).toBe(true);
    const head = box.querySelector(".omfp-bbox__head");
    expect([...(head?.children ?? [])].map((part) => part.className)).toEqual([
      "omfp-bbox__label",
      "omfp-bbox__line",
      "omfp-bbox__end",
    ]);
    expect(screen.getByRole("heading", { level: 3 }).id).toBe("repos");
    expect(head?.querySelector(".omfp-bbox__end")?.textContent).toBe("0:42");
    expect(screen.getByTestId("content").parentElement?.className).toBe("omfp-bbox__body");
  });

  // spec: UI-187
  test("a labelled rule is a heading that opens the next section", () => {
    render(<BoxRule label="Output" data-testid="rule" />);
    const rule = screen.getByRole("heading", { level: 3, name: "Output" });
    expect(rule === screen.getByTestId("rule")).toBe(true);
    expect(rule.className).toBe("omfp-bbox__rule");
  });
});

describe("HelpDialog", () => {
  // spec: UI-198, UI-187, UI-092
  test("a modal border box titled help, its sections on labelled rules; Escape asks to close", async () => {
    const onClose = mock(() => {});
    render(
      <HelpDialog label="Keys and commands" filter={<input aria-label="Filter" />} end="esc" onClose={onClose}>
        <HelpSection title="normal" note="vim keys">
          <HelpRow keys={["spc a", "] a"]} description="Next thread needing attention" />
          <HelpRow keys={[":theme"]} args="system | light | dark" description="Colour scheme" />
        </HelpSection>
      </HelpDialog>,
    );
    const dialog = screen.getByRole("dialog", { name: "Keys and commands" });
    expect(dialog.querySelector(".omfp-bbox__label")?.textContent).toBe("help");
    expect(dialog.querySelector(".omfp-bbox__end")?.textContent).toBe("esc");
    expect(screen.getByRole("heading", { level: 2, name: "help" }).className).toBe("omfp-bbox__label");
    expect(screen.getByRole("heading", { level: 3, name: "normal" }).className).toBe("omfp-bbox__rule");
    expect([...dialog.querySelectorAll(".omfp-help__row")].map((row) => row.textContent)).toEqual([
      "spc a] aNext thread needing attention",
      ":themesystem | light | darkColour scheme",
    ]);
    expect([...dialog.querySelectorAll(".omfp-help__row kbd")].map((chip) => chip.className)).toEqual([
      "omfp-kbd",
      "omfp-kbd",
      "omfp-kbd",
    ]);
    // The scrolling list is a Tab stop, so the keyboard can scroll it.
    expect(screen.getByRole("region", { name: "Keys and commands" }).getAttribute("tabindex")).toBe("0");
    await userEvent.setup().keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("approval and ask cards", () => {
  // spec: UI-164
  test("the countdown turns low; a resolved line keeps the question and the answer", () => {
    render(
      <>
        <InteractionCard title="Run a command?" meta="1/2" countdown="0:04" low data-testid="card" />
        <ResolvedLine outcome="denied" question="Run a command?" answer="Deny" data-testid="resolved" />
      </>,
    );
    expect(screen.getByTestId("card").querySelector("[data-low]")?.textContent).toBe("0:04");
    const resolved = screen.getByTestId("resolved");
    expect(resolved.getAttribute("data-outcome")).toBe("denied");
    expect(resolved.querySelector("[data-icon=x]")).not.toBeNull();
    expect(resolved.textContent).toBe("Run a command?Deny");
  });

  // spec: UI-164, UI-187
  test("the `◆` title sits on the top border, the step and countdown at its right end", () => {
    render(
      <InteractionCard title="Run a command?" meta="1/2" countdown="0:42" data-testid="card">
        <button type="button">Approve</button>
      </InteractionCard>,
    );
    const card = screen.getByTestId("card");
    expect(card.classList.contains("omfp-bbox")).toBe(true);
    expect(card.querySelector(".omfp-bbox__label")?.textContent).toBe("◆ Run a command?");
    expect(card.querySelector(".omfp-bbox__end")?.textContent).toBe("1/20:42");
    expect(screen.getByRole("button", { name: "Approve" }).parentElement?.classList.contains("omfp-bbox__body")).toBe(
      true,
    );
  });
});

/** A workspace section that toggles itself. */
function ToggledSection() {
  const [expanded, setExpanded] = useState(true);
  return (
    <WorkspaceSection
      name="omfp"
      path="~/ws/omfp"
      count={3}
      current
      currentLabel="current"
      expanded={expanded}
      onExpandedChange={setExpanded}
      toggleLabel={expanded ? "Collapse omfp" : "Expand omfp"}
      toggleProps={{ "data-testid": "toggle" }}
      nameProps={{ "data-testid": "name" }}
      actions={<button type="button">Remove</button>}
      data-testid="section"
    >
      <span data-testid="rows">rows</span>
    </WorkspaceSection>
  );
}

/** A thinking block that toggles itself. */
function ToggledThinking() {
  const [expanded, setExpanded] = useState(false);
  return (
    <Thinking label="Thinking" meta="4s" expanded={expanded} onExpandedChange={setExpanded}>
      Considering the options.
    </Thinking>
  );
}

describe("shell", () => {
  // spec: UI-167
  test("the mode line shows the mode pill and its segments", () => {
    render(
      <ModeLine mode="insert" data-testid="line">
        <ModeSegment kind="title">Fix it</ModeSegment>
        <ModeSegment kind="error">No such command</ModeSegment>
      </ModeLine>,
    );
    const line = screen.getByTestId("line");
    expect(line.querySelector(".omfp-mode-pill")?.textContent).toBe("INSERT");
    expect([...line.querySelectorAll("[data-kind]")].map((segment) => segment.getAttribute("data-kind"))).toEqual([
      "title",
      "error",
    ]);
  });

  // spec: UI-193
  test("a mode-line notice sits after the segments with its tone, count and key chip, and opens on press", async () => {
    const user = userEvent.setup();
    const onPress = mock(() => undefined);
    render(
      <ModeLine
        mode="normal"
        data-testid="line"
        notice={<ModeNotice tone="error" title="flaky" body="failed" more={2} kbd="g n" onPress={onPress} />}
      >
        <ModeSegment kind="title">Fix it</ModeSegment>
      </ModeLine>,
    );
    const line = screen.getByTestId("line");
    expect([...line.children].map((child) => child.className)).toEqual([
      "omfp-mode-pill",
      "omfp-modeline__segments",
      "omfp-modeline__notice",
    ]);
    const notice = screen.getByRole("button", { name: "flaky failed +2" });
    expect(notice.getAttribute("data-tone")).toBe("error");
    expect(notice.querySelector("kbd")?.textContent).toBe("g n");
    await user.click(notice);
    expect(onPress).toHaveBeenCalledTimes(1);
  });

  // spec: UI-441, UI-167
  test("the mode line's view tabs sit between the segments and the notice; the pill takes its own props", async () => {
    const user = userEvent.setup();
    const onSelect = mock((_id: string) => undefined);
    render(
      <ModeLine
        mode="normal"
        data-testid="line"
        modeLabel="VISUAL"
        pillProps={{ "data-testid": "pill", "data-vim": "visual" }}
        views={
          <ModeViews
            label="Views"
            views={[
              { id: "default", title: "Default" },
              { id: "review", title: "Review" },
            ]}
            current="review"
            onSelect={onSelect}
          />
        }
        notice={<ModeNotice tone="info" title="CI" body="is green" />}
      />,
    );
    expect([...screen.getByTestId("line").children].map((child) => child.className)).toEqual([
      "omfp-mode-pill",
      "omfp-modeline__segments",
      "omfp-modeline__views",
      "omfp-modeline__notice",
    ]);
    expect([screen.getByTestId("pill").textContent, screen.getByTestId("pill").dataset.vim]).toEqual([
      "VISUAL",
      "visual",
    ]);
    const tabs = within(screen.getByRole("navigation", { name: "Views" })).getAllByRole("button");
    expect(tabs.map((tab) => [tab.textContent, tab.getAttribute("aria-current"), tab.dataset.view])).toEqual([
      ["Default", null, "default"],
      ["Review", "true", "review"],
    ]);
    await user.click(tabs[0] as HTMLElement);
    expect(onSelect.mock.calls).toEqual([["default"]]);
  });

  // spec: UI-168
  test("a completion bolds the matched part and is selected for the ▸", () => {
    render(<CommandItem command="rescan" matched={2} description="Rescan" selected data-testid="item" />);
    const item = screen.getByTestId("item");
    expect(item.getAttribute("aria-selected")).toBe("true");
    expect(item.querySelector("b")?.textContent).toBe("re");
  });

  // spec: UI-169, UI-442
  test("a thread row is a list item: a select button with its mark, title, unread word, time and snippet, then its actions and delete", async () => {
    const user = userEvent.setup();
    const onSelect = mock(() => undefined);
    const onDelete = mock(() => undefined);
    render(
      <RowList>
        <ThreadRow
          state="awaiting"
          stateLabel="awaiting input"
          title="Fix it"
          meta="2m"
          selected
          fresh
          unread
          unreadLabel="unread"
          archived
          snippet="…the failing test…"
          onSelect={onSelect}
          selectProps={{ "data-testid": "select" }}
          actions={<button type="button">Unarchive</button>}
          onDelete={onDelete}
          deleteProps={{ "data-testid": "delete" }}
          data-testid="row"
        />
      </RowList>,
    );
    const row = screen.getByRole("listitem");
    expect(row.getAttribute("data-testid")).toBe("row");
    expect([row.dataset.state, row.dataset.selected, row.dataset.fresh, row.dataset.unread]).toEqual([
      "awaiting",
      "true",
      "true",
      "true",
    ]);
    expect([row.dataset.archived, row.dataset.snippet]).toEqual(["true", "true"]);
    const select = screen.getByTestId("select");
    expect(select.getAttribute("aria-current")).toBe("true");
    expect(select.textContent).toBe("Fix itunread2m…the failing test…");
    expect(within(select).getByTestId("thread-status").getAttribute("aria-label")).toBe("awaiting input");
    expect(within(select).getByTestId("unread").classList.contains("omfp-visually-hidden")).toBe(true);
    expect(within(select).getByTestId("thread-snippet").textContent).toBe("…the failing test…");
    expect([...row.children].map((child) => child.textContent)).toEqual([select.textContent, "Unarchive", ""]);
    await user.click(select);
    await user.click(screen.getByRole("button", { name: "Delete thread" }));
    expect([onSelect.mock.calls.length, onDelete.mock.calls.length]).toEqual([1, 1]);
    expect(screen.getByTestId("delete").classList.contains("omfp-row__delete")).toBe(true);
  });

  // spec: UI-169, UI-442
  test("a placeholder title is italic; without onDelete there is no delete", () => {
    render(
      <RowList>
        <ThreadRow state="idle" title="Untitled" placeholder meta="draft" />
      </RowList>,
    );
    expect(screen.getByText("Untitled").dataset.placeholder).toBe("true");
    expect(screen.queryByRole("button", { name: "Delete thread" })).toBeNull();
  });

  // spec: UI-170, UI-339, UI-442
  test("a workspace head: the ▾/▸ toggle, the name as a heading with its path as tooltip and description, the count, the actions", async () => {
    const user = userEvent.setup();
    render(<ToggledSection />);
    const section = screen.getByTestId("section");
    expect([section.dataset.expanded, section.dataset.current]).toEqual(["true", "true"]);
    const toggle = screen.getByRole("button", { name: "Collapse omfp" });
    expect([toggle.getAttribute("data-testid"), toggle.textContent]).toEqual(["toggle", "▾"]);
    const name = screen.getByRole("heading", { level: 2 });
    expect([name.getAttribute("data-testid"), name.textContent, name.classList.contains("omfp-grad-text")]).toEqual([
      "name",
      "omfp (current)",
      true,
    ]);
    expect([name.getAttribute("title"), name.getAttribute("aria-description")]).toEqual(["~/ws/omfp", "~/ws/omfp"]);
    expect(section.getAttribute("aria-labelledby")).toBe(name.id);
    const head = section.querySelector(".omfp-wsection__head");
    expect(head?.textContent).toBe("▾omfp (current)3Remove");
    expect(head?.querySelector(".omfp-wsection__actions")?.textContent).toBe("Remove");
    await user.click(toggle);
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(screen.getByRole("button", { name: "Expand omfp" }).textContent).toBe("▸");
    expect(screen.queryByTestId("rows")).toBeNull();
  });

  // spec: UI-442
  test("a workspace's rename field replaces its name", () => {
    render(<WorkspaceSection name="omfp" expanded editor={<input aria-label="New name" />} />);
    expect(screen.queryByRole("heading")).toBeNull();
    expect(screen.getByRole("textbox", { name: "New name" })).toBeDefined();
  });

  // spec: UI-169, UI-442
  test("a toggle row: the ▾/▸ glyph where a thread row's mark sits, its label, then its end; aria-expanded", async () => {
    const user = userEvent.setup();
    const onPress = mock(() => undefined);
    const { rerender } = render(
      <ToggleRow expanded={false} end={<span>●1</span>} onPress={onPress} data-testid="toggle">
        Jobs (3)
      </ToggleRow>,
    );
    const toggle = screen.getByTestId("toggle");
    expect([...toggle.children].map((child) => [child.className, child.textContent])).toEqual([
      ["omfp-toggle-row__glyph", "▸"],
      ["omfp-row__title", "Jobs (3)"],
      ["omfp-toggle-row__end", "●1"],
    ]);
    expect([toggle.classList.contains("omfp-row"), toggle.getAttribute("aria-expanded")]).toEqual([true, "false"]);
    expect(toggle.querySelector(".omfp-toggle-row__glyph")?.getAttribute("aria-hidden")).toBe("true");
    await user.click(toggle);
    expect(onPress).toHaveBeenCalledTimes(1);
    rerender(<ToggleRow expanded data-testid="toggle" />);
    expect([toggle.getAttribute("aria-expanded"), toggle.textContent]).toEqual(["true", "▾"]);
  });

  // spec: UI-169, UI-442
  test("a nested row is a list item: its select button with the mark, title and meta, then its actions", async () => {
    const user = userEvent.setup();
    const onSelect = mock(() => undefined);
    render(
      <RowList>
        <NestedRow
          state="failed"
          stateLabel="failed"
          title="deploy"
          meta="failed"
          onSelect={onSelect}
          selectProps={{ "data-testid": "select" }}
          actions={<button type="button">Run deploy</button>}
          data-testid="row"
        />
      </RowList>,
    );
    const row = screen.getByRole("listitem");
    expect([row.getAttribute("data-testid"), row.dataset.state, row.classList.contains("omfp-nested-row")]).toEqual([
      "row",
      "failed",
      true,
    ]);
    const select = screen.getByTestId("select");
    // The mark's name says the state; the meta, which actions replace on hover and focus, is visual only.
    expect(select.querySelector("[data-state]")?.getAttribute("aria-label")).toBe("failed");
    expect(select.querySelector(".omfp-row__meta")?.getAttribute("aria-hidden")).toBe("true");
    expect([
      select.querySelector(".omfp-row__title")?.textContent,
      select.querySelector(".omfp-row__meta")?.textContent,
    ]).toEqual(["deploy", "failed"]);
    expect([...row.children].map((child) => child.className)).toEqual([
      "omfp-nested-row__select omfp-focus-item__target",
      "omfp-nested-row__actions",
    ]);
    await user.click(select);
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  // spec: UI-171
  test("a thinking block expands to its text", async () => {
    const user = userEvent.setup();
    render(<ToggledThinking />);
    expect(screen.queryByText("Considering the options.")).toBeNull();
    await user.click(screen.getByTestId("thinking-head"));
    expect(screen.getByText("Considering the options.")).toBeDefined();
  });

  // spec: UI-171, UI-471
  test("a message that continues its turn has no name line and is no article: the turn's first message is", () => {
    render(
      <>
        <Message author="assistant" name="Assistant" data-testid="first">
          one
        </Message>
        <Message author="assistant" name="Assistant" continues data-testid="next" error="stopped">
          two
        </Message>
      </>,
    );
    const [first, next] = [screen.getByTestId("first"), screen.getByTestId("next")];
    expect([first.tagName, first.querySelector(".omfp-msg__name")?.textContent]).toEqual(["ARTICLE", "◆ Assistant"]);
    expect([next.tagName, next.querySelectorAll(".omfp-msg__name").length, next.textContent]).toEqual([
      "DIV",
      0,
      "twostopped",
    ]);
    expect(screen.getAllByRole("article").length).toBe(1);
  });
});

/** A path field completing in a fixed folder. */
function CompletingPathField() {
  const [value, setValue] = useState("");
  return (
    <PathField
      label="Folder"
      value={value}
      onChange={setValue}
      complete={async () => [
        { name: "pmo", path: "/home/u/projects/pmo", isGitRoot: true },
        { name: "papers", path: "/home/u/projects/papers", isGitRoot: false },
      ]}
      data-testid="path"
    />
  );
}

describe("path field", () => {
  // spec: UI-151
  test("a mono field with a folder icon and a tab chip; completions mute the typed folder and bold the match", async () => {
    const user = userEvent.setup();
    render(<CompletingPathField />);
    const box = document.querySelector(".omfp-input-box");
    expect(box?.getAttribute("data-mono")).toBe("true");
    expect(box?.querySelector("[data-icon=folder-open]")).not.toBeNull();
    expect(box?.querySelector("kbd")?.textContent).toBe("tab");
    await user.type(screen.getByTestId("path"), "~/projects/p");
    const options = await screen.findAllByTestId("path-option");
    expect(options.map((option) => option.querySelector(".omfp-path-field__dir")?.textContent)).toEqual([
      "~/projects/",
      "~/projects/",
    ]);
    expect(options.map((option) => option.querySelector("b")?.textContent)).toEqual(["p", "p"]);
    expect(options.map((option) => option.querySelector(".omfp-path-field__meta")?.textContent)).toEqual([
      "git",
      "folder",
    ]);
  });
});

describe("cards and code", () => {
  // spec: UI-162
  test("a card has a head row and a body", () => {
    render(
      <Card title="Card" actions={<span>3</span>} data-testid="card">
        body
      </Card>,
    );
    const card = screen.getByTestId("card");
    expect(card.querySelector(".omfp-card__header")?.textContent).toBe("Card3");
    expect(card.querySelector(".omfp-card__body")?.textContent).toBe("body");
  });

  // spec: UI-166
  test("a code block names its language for the top-right corner, fenced Markdown too", () => {
    render(
      <>
        <Code block language="ts" code="const a = 1;" data-testid="block" />
        <Markdown text={"```rust\nfn main() {}\n```"} data-testid="markdown" />
      </>,
    );
    expect(screen.getByTestId("block").getAttribute("data-lang")).toBe("ts");
    expect(screen.getByTestId("block").textContent).toBe("const a = 1;");
    expect(screen.getByTestId("markdown").querySelector("pre")?.getAttribute("data-lang")).toBe("rust");
  });

  // spec: UI-147
  test("the stream caret is decorative", () => {
    render(<StreamCaret data-testid="caret" />);
    expect(screen.getByTestId("caret").getAttribute("aria-hidden")).toBe("true");
  });
});

describe("ChatHeader", () => {
  // spec: UI-443, UI-171
  test("a title row, then the bar; the title is the page's heading, in gradient text unless a placeholder", () => {
    const { rerender } = render(
      <ChatHeader data-testid="header" bar={<span>model</span>}>
        <ChatTitle data-testid="title">Fix it</ChatTitle>
        <span>running</span>
      </ChatHeader>,
    );
    const header = screen.getByTestId("header");
    expect([...header.children].map((child) => [child.className, child.textContent])).toEqual([
      ["omfp-chat-header__top", "Fix itrunning"],
      ["omfp-chat-header__bar", "model"],
    ]);
    const title = screen.getByRole("heading", { level: 1, name: "Fix it" });
    expect([title.getAttribute("data-testid"), title.classList.contains("omfp-grad-text")]).toEqual(["title", true]);
    rerender(
      <ChatHeader data-testid="header">
        <ChatTitle placeholder>Untitled</ChatTitle>
      </ChatHeader>,
    );
    const placeholder = screen.getByRole("heading", { level: 1, name: "Untitled" });
    expect([placeholder.dataset.placeholder, placeholder.classList.contains("omfp-grad-text")]).toEqual([
      "true",
      false,
    ]);
    expect(screen.getByTestId("header").querySelector(".omfp-chat-header__bar")).toBeNull();
  });
});

describe("ComposerBar", () => {
  // spec: UI-480, UI-172
  test("the hints in a group at its start, the actions in a group at its end, in their order", () => {
    const { rerender } = render(
      <ComposerBar data-testid="bar" hints={<ComposerHint>esc normal</ComposerHint>}>
        <button type="button">Abort</button>
        <button type="submit">Send</button>
      </ComposerBar>,
    );
    const bar = screen.getByTestId("bar");
    expect([...bar.children].map((child) => [child.className, child.textContent])).toEqual([
      ["omfp-compose__hints", "esc normal"],
      ["omfp-compose__actions", "AbortSend"],
    ]);
    rerender(
      <ComposerBar data-testid="bar">
        <button type="submit">Send</button>
      </ComposerBar>,
    );
    expect([...screen.getByTestId("bar").children].map((child) => child.className)).toEqual(["omfp-compose__actions"]);
  });
});

describe("QueueChip", () => {
  // spec: UI-445, UI-172
  test("a queued message in the queue list: its kind, its text, promote and remove, both off while busy", async () => {
    const user = userEvent.setup();
    const onPromote = mock(() => undefined);
    const onRemove = mock(() => undefined);
    const { rerender } = render(
      <QueueList aria-label="Queued messages" data-testid="queue">
        <QueueChip kind="follow-up" onPromote={onPromote} onRemove={onRemove} data-testid="chip">
          later please
        </QueueChip>
      </QueueList>,
    );
    const list = screen.getByRole("list", { name: "Queued messages" });
    expect(list.getAttribute("data-testid")).toBe("queue");
    const chip = screen.getByRole("listitem");
    expect([chip.getAttribute("data-testid"), chip.dataset.kind, chip.getAttribute("aria-busy")]).toEqual([
      "chip",
      "follow-up",
      "false",
    ]);
    expect(chip.querySelector(".omfp-queue-chip__text")?.textContent).toBe("later please");
    await user.click(screen.getByTestId("queue-promote"));
    await user.click(screen.getByRole("button", { name: "Remove" }));
    expect([onPromote.mock.calls.length, onRemove.mock.calls.length]).toEqual([1, 1]);
    rerender(
      <QueueList aria-label="Queued messages">
        <QueueChip kind="steer" busy onPromote={onPromote} onRemove={onRemove}>
          now
        </QueueChip>
      </QueueList>,
    );
    expect(screen.getByRole("listitem").getAttribute("aria-busy")).toBe("true");
    expect(screen.getByTestId("queue-promote").hasAttribute("disabled")).toBe(true);
    expect(screen.getByTestId("queue-remove").hasAttribute("disabled")).toBe(true);
  });
});
