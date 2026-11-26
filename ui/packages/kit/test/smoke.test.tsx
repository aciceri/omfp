import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import type { ReactElement } from "react";
import {
  ActionRow,
  ApprovalCommand,
  Badge,
  BorderBox,
  BoxRule,
  Brand,
  Button,
  Card,
  ChatHeader,
  ChatTitle,
  Checkbox,
  Chip,
  Code,
  CommandItem,
  CommandLine,
  CommandMenu,
  Composer,
  Count,
  DiffStat,
  Disclosure,
  EmptyState,
  ExitCode,
  HelpDialog,
  HelpRow,
  HelpSection,
  HintRow,
  Icon,
  IconButton,
  InteractionCard,
  Kbd,
  KindMark,
  ListBox,
  ListBoxItem,
  LiveRegion,
  Log,
  Markdown,
  Menu,
  MenuItem,
  MenuSection,
  Message,
  ModeLine,
  ModeNotice,
  ModePill,
  ModeSegment,
  NestedRow,
  Progress,
  QueueChip,
  Radio,
  RadioGroup,
  ResolvedLine,
  SearchField,
  Select,
  Separator,
  StateBadge,
  StateMark,
  StreamCaret,
  Tab,
  TabList,
  TabPanel,
  Tabs,
  TextArea,
  TextField,
  Thinking,
  ThreadRow,
  Toast,
  ToastRegion,
  TodoList,
  ToggleRow,
  Toolbar,
  ToolCard,
  WorkspaceSection,
} from "../src/index.ts";

afterEach(cleanup);

/** Common forwarded props: every component puts them on its root element. */
const FORWARDED = { "data-testid": "subject", id: "subject-id", className: "extra" } as const;

interface Case {
  readonly name: string;
  readonly element: ReactElement;
  /** The kit class the root carries. */
  readonly base: string;
  /** The root's ARIA role, when it has one. */
  readonly role?: string;
  /** Fields put `id` on their input (so labels and `aria-*` refer to it), not on the wrapper. */
  readonly idOnControl?: true;
}

const CASES: readonly Case[] = [
  { name: "Button", element: <Button {...FORWARDED}>Go</Button>, base: "omfp-button", role: "button" },
  {
    name: "IconButton",
    element: <IconButton {...FORWARDED} label="Close" icon="×" />,
    base: "omfp-icon-button",
    role: "button",
  },
  { name: "Card", element: <Card {...FORWARDED} title="Title" />, base: "omfp-card" },
  {
    name: "Tabs",
    element: (
      <Tabs {...FORWARDED}>
        <TabList aria-label="Views">
          <Tab id="a">A</Tab>
        </TabList>
        <TabPanel id="a">Panel</TabPanel>
      </Tabs>
    ),
    base: "omfp-tabs",
  },
  {
    name: "ListBox",
    element: (
      <ListBox {...FORWARDED} aria-label="Items">
        <ListBoxItem id="one">One</ListBoxItem>
      </ListBox>
    ),
    base: "omfp-listbox",
    role: "listbox",
  },
  {
    name: "Menu",
    element: (
      <Menu {...FORWARDED} aria-label="Actions">
        <MenuSection title="Thread">
          <MenuItem id="rename">Rename</MenuItem>
        </MenuSection>
        <Separator />
        <MenuItem id="delete">Delete</MenuItem>
      </Menu>
    ),
    base: "omfp-menu",
    role: "menu",
  },
  { name: "TextField", element: <TextField {...FORWARDED} label="Name" />, base: "omfp-field", idOnControl: true },
  { name: "TextArea", element: <TextArea {...FORWARDED} label="Message" />, base: "omfp-field", idOnControl: true },
  {
    name: "SearchField",
    element: <SearchField {...FORWARDED} label="Filter" />,
    base: "omfp-search",
    idOnControl: true,
  },
  {
    name: "Select",
    element: (
      <Select
        {...FORWARDED}
        label="Mode"
        items={[{ id: "yolo", label: "Yolo" }]}
        selectedKey="yolo"
        onSelectionChange={() => undefined}
      />
    ),
    base: "omfp-select",
    idOnControl: true,
  },
  {
    name: "Checkbox",
    element: <Checkbox {...FORWARDED}>Pinned</Checkbox>,
    base: "omfp-checkbox",
    idOnControl: true,
  },
  {
    name: "RadioGroup",
    element: (
      <RadioGroup {...FORWARDED} label="Scheme">
        <Radio value="light">Light</Radio>
      </RadioGroup>
    ),
    base: "omfp-radio-group",
    role: "radiogroup",
  },
  { name: "Badge", element: <Badge {...FORWARDED}>3</Badge>, base: "omfp-badge" },
  { name: "Chip", element: <Chip {...FORWARDED}>file.png</Chip>, base: "omfp-chip" },
  { name: "Kbd", element: <Kbd {...FORWARDED}>j</Kbd>, base: "omfp-kbd" },
  {
    name: "Progress",
    element: <Progress {...FORWARDED} label="Loading" />,
    base: "omfp-progress",
    role: "progressbar",
  },
  { name: "Log", element: <Log {...FORWARDED} lines={["a", "b"]} />, base: "omfp-log", role: "log" },
  { name: "Markdown", element: <Markdown {...FORWARDED} text="*hi*" />, base: "omfp-markdown" },
  { name: "Code", element: <Code {...FORWARDED} code="ls" />, base: "omfp-code" },
  {
    name: "LiveRegion",
    element: <LiveRegion {...FORWARDED} message="Saved" />,
    base: "omfp-visually-hidden",
    role: "status",
  },
  {
    name: "Toolbar",
    element: (
      <Toolbar {...FORWARDED} aria-label="Format">
        <Button>B</Button>
      </Toolbar>
    ),
    base: "omfp-toolbar",
    role: "toolbar",
  },
  { name: "Disclosure", element: <Disclosure {...FORWARDED} title="Thinking" />, base: "omfp-disclosure" },
  { name: "Icon", element: <Icon {...FORWARDED} name="check" />, base: "omfp-ic" },
  {
    name: "StateMark",
    element: <StateMark {...FORWARDED} state="running" label="Running" />,
    base: "omfp-state-mark",
    role: "img",
  },
  { name: "StateBadge", element: <StateBadge {...FORWARDED} state="awaiting" />, base: "omfp-state-badge" },
  { name: "Count", element: <Count {...FORWARDED}>12</Count>, base: "omfp-count" },
  { name: "ExitCode", element: <ExitCode {...FORWARDED} code={1} />, base: "omfp-exit" },
  { name: "KindMark", element: <KindMark {...FORWARDED} kind="worktree" />, base: "omfp-kind" },
  { name: "DiffStat", element: <DiffStat {...FORWARDED} added={3} removed={1} />, base: "omfp-diffstat" },
  { name: "Toast", element: <Toast {...FORWARDED}>Saved</Toast>, base: "omfp-toast", role: "status" },
  { name: "ToastRegion", element: <ToastRegion {...FORWARDED} />, base: "omfp-toasts" },
  { name: "EmptyState", element: <EmptyState {...FORWARDED} icon="inbox" title="No threads" />, base: "omfp-empty" },
  { name: "ToolCard", element: <ToolCard {...FORWARDED} status="done" name="bash" />, base: "omfp-tool" },
  { name: "TodoList", element: <TodoList {...FORWARDED} items={[]} />, base: "omfp-todo" },
  { name: "InteractionCard", element: <InteractionCard {...FORWARDED} title="Run?" />, base: "omfp-request" },
  { name: "BorderBox", element: <BorderBox {...FORWARDED} title="Repositories" />, base: "omfp-bbox" },
  { name: "BoxRule", element: <BoxRule {...FORWARDED} label="Output" />, base: "omfp-bbox__rule" },
  { name: "ApprovalCommand", element: <ApprovalCommand {...FORWARDED} command="ls" />, base: "omfp-request__code" },
  {
    name: "ResolvedLine",
    element: <ResolvedLine {...FORWARDED} outcome="approved" question="Run?" answer="yes" />,
    base: "omfp-resolved",
  },
  { name: "ModePill", element: <ModePill {...FORWARDED} mode="insert" />, base: "omfp-mode-pill" },
  { name: "ModeLine", element: <ModeLine {...FORWARDED} mode="threads" />, base: "omfp-modeline" },
  { name: "ModeSegment", element: <ModeSegment {...FORWARDED}>opus</ModeSegment>, base: "omfp-modeline__segment" },
  {
    name: "ModeNotice",
    element: <ModeNotice {...FORWARDED} tone="attention" title="Fix it" body="needs you" />,
    base: "omfp-modeline__notice",
    role: "button",
  },
  { name: "CommandLine", element: <CommandLine {...FORWARDED} />, base: "omfp-cmd" },
  { name: "CommandMenu", element: <CommandMenu {...FORWARDED} />, base: "omfp-cmdmenu" },
  {
    name: "CommandItem",
    element: <CommandItem {...FORWARDED} command="rescan" />,
    base: "omfp-cmdmenu__item",
    role: "option",
  },
  {
    name: "HelpDialog",
    element: <HelpDialog {...FORWARDED} label="Keys" filter={null} inline />,
    base: "omfp-help",
  },
  { name: "HelpSection", element: <HelpSection {...FORWARDED} title="threads" />, base: "omfp-help__section" },
  {
    name: "HelpRow",
    element: <HelpRow {...FORWARDED} keys={["j"]} description="Next thread" />,
    base: "omfp-help__row",
  },
  { name: "Brand", element: <Brand {...FORWARDED} host="devbox" />, base: "omfp-brand" },
  {
    name: "WorkspaceSection",
    element: <WorkspaceSection {...FORWARDED} name="omfp" expanded />,
    base: "omfp-wsection",
  },
  { name: "ThreadRow", element: <ThreadRow {...FORWARDED} state="idle" title="Fix it" />, base: "omfp-thread" },
  { name: "ActionRow", element: <ActionRow {...FORWARDED}>thread</ActionRow>, base: "omfp-action-row", role: "button" },
  {
    name: "ToggleRow",
    element: (
      <ToggleRow {...FORWARDED} expanded={false}>
        Archived (2)
      </ToggleRow>
    ),
    base: "omfp-toggle-row",
    role: "button",
  },
  { name: "NestedRow", element: <NestedRow {...FORWARDED} state="idle" title="deploy" />, base: "omfp-nested-row" },
  { name: "HintRow", element: <HintRow {...FORWARDED} />, base: "omfp-hint-row" },
  {
    name: "ChatHeader",
    element: (
      <ChatHeader {...FORWARDED}>
        <ChatTitle>Fix it</ChatTitle>
      </ChatHeader>
    ),
    base: "omfp-chat-header",
  },
  { name: "ChatTitle", element: <ChatTitle {...FORWARDED}>Fix it</ChatTitle>, base: "omfp-chat-header__title" },
  { name: "Message", element: <Message {...FORWARDED} author="user" name="you" />, base: "omfp-msg" },
  { name: "StreamCaret", element: <StreamCaret {...FORWARDED} />, base: "omfp-caret" },
  {
    name: "Thinking",
    element: <Thinking {...FORWARDED} label="Thinking" expanded={false} />,
    base: "omfp-reasoning",
  },
  { name: "Composer", element: <Composer {...FORWARDED} />, base: "omfp-compose" },
  {
    name: "QueueChip",
    element: (
      <QueueChip {...FORWARDED} kind="steer">
        later
      </QueueChip>
    ),
    base: "omfp-queue-chip",
  },
];

describe("kit components", () => {
  // spec: UI-093, UI-131, UI-157, UI-160, UI-161, UI-163, UI-164, UI-167, UI-168, UI-169, UI-170, UI-171, UI-172, UI-187, UI-193, UI-198
  test.each(CASES.map((c) => [c.name, c] as const))("%s renders and forwards id, className, data-*", (_name, c) => {
    render(c.element);
    const root = screen.getByTestId("subject");
    const withId = document.getElementById("subject-id");
    expect(c.idOnControl === true ? withId !== root && root.contains(withId) : withId === root).toBe(true);
    expect(root.classList.contains(c.base)).toBe(true);
    expect(root.classList.contains("extra")).toBe(true);
    if (c.role !== undefined) {
      expect(root.getAttribute("role") ?? (root.tagName === "BUTTON" ? "button" : null)).toBe(c.role);
    }
  });
});
