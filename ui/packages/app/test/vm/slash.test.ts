import { describe, expect, test } from "bun:test";
import type { AvailableSlashCommand } from "@omfp/core";
import { slashSuggestions } from "../../src/vm/composer.ts";
import { OMP_TUI_COMMANDS, slashAction, slashEntries } from "../../src/vm/slash.ts";
import { fixture } from "../widgets/chat/fixtures.ts";

/** The commands the pinned omp announces, as recorded (`available_commands_update`). */
const ANNOUNCED = fixture("text_reply").commands;

const action = (draft: string, commands: readonly AvailableSlashCommand[] = ANNOUNCED) => slashAction(draft, commands);

describe("slash commands over omp's RPC", () => {
  // spec: UI-975
  test("the TUI-only table names no command the pinned omp announces; the routed builtins are announced", () => {
    const announced = new Set(ANNOUNCED.flatMap((command) => [command.name, ...(command.aliases ?? [])]));
    expect(announced.size).toBeGreaterThan(40);
    expect(
      OMP_TUI_COMMANDS.flatMap((command) => [command.name, ...command.aliases]).filter((name) => announced.has(name)),
    ).toEqual([]);
    const builtins = ANNOUNCED.filter((command) => command.source === "builtin").map((command) => command.name);
    expect(["model", "compact", "export"].filter((name) => !builtins.includes(name))).toEqual([]);
  });

  // spec: UI-975
  test("a draft that is no command, or a command omp runs, is sent", () => {
    expect(action("hello")).toEqual({ type: "send" });
    expect(action("/")).toEqual({ type: "send" });
    expect(action("/home/me/notes.md explain this")).toEqual({ type: "send" });
    expect(action("/dirs")).toEqual({ type: "send", command: "dirs" });
    expect(action("/mcp reconnect github")).toEqual({ type: "send", command: "mcp" });
    expect(action("/worktree feature")).toEqual({ type: "send", command: "wt" });
  });

  // spec: UI-976
  test("routed builtins run their omfp equivalent", () => {
    const route = (draft: string) => {
      const result = action(draft);
      return result.type === "route" ? [result.name, result.route] : result;
    };
    expect(route("/login")).toEqual(["login", { type: "command", id: "auth.login", args: [] }]);
    expect(route("/login  anthropic ")).toEqual(["login", { type: "command", id: "auth.login", args: ["anthropic"] }]);
    expect(route("/logout openai")).toEqual(["logout", { type: "command", id: "auth.logout", args: ["openai"] }]);
    expect(route("/model")).toEqual(["model", { type: "command", id: "thread.pickModel", args: [] }]);
    expect(route("/models")).toEqual(["model", { type: "command", id: "thread.pickModel", args: [] }]);
    expect(route("/compact")).toEqual(["compact", { type: "command", id: "thread.compact", args: [] }]);
    expect(route("/compact keep the test plan")).toEqual([
      "compact",
      { type: "command", id: "thread.compact", args: ["keep the test plan"] },
    ]);
    expect(route("/compact soft keep the plan")).toEqual([
      "compact",
      { type: "command", id: "thread.compact", args: ["soft keep the plan"] },
    ]);
    expect(route("/fork")).toEqual(["fork", { type: "fork" }]);
    expect(route("/copy")).toEqual(["copy", { type: "command", id: "thread.copy", args: [] }]);
    expect(route("/export")).toEqual(["export", { type: "command", id: "thread.export", args: [] }]);
    expect(route("/new")).toEqual(["new", { type: "command", id: "thread.new", args: [] }]);
    expect(route("/hotkeys")).toEqual(["hotkeys", { type: "command", id: "help.open", args: [] }]);
    expect(route("/resume")).toEqual(["resume", { type: "command", id: "threads.pick", args: [] }]);
    expect(route("/live")).toEqual(["live", { type: "command", id: "live.toggle", args: [] }]);
  });

  // spec: UI-976
  test("arguments omfp has no equivalent for go to omp when omp runs the command", () => {
    expect(action("/model anthropic/claude-sonnet-4-5")).toEqual({ type: "send", command: "model" });
    expect(action("/export out.html")).toEqual({ type: "send", command: "export" });
  });

  // spec: UI-975, UI-976
  test("a command the user, a project or a plugin defines goes to omp whatever its name", () => {
    const own: AvailableSlashCommand[] = [
      ...ANNOUNCED,
      { name: "settings", source: "file", description: "Project settings" },
      { name: "login", source: "custom" },
    ];
    expect(action("/settings", own)).toEqual({ type: "send", command: "settings" });
    expect(action("/login", own)).toEqual({ type: "send", command: "login" });
  });

  // spec: UI-977
  test("TUI-only builtins without an equivalent are refused with the reason", () => {
    expect(action("/settings")).toEqual({
      type: "refuse",
      message: "/settings was not sent: Needs oh-my-pi's terminal UI",
    });
    expect(action("/q")).toEqual({
      type: "refuse",
      message: "/quit was not sent: Needs oh-my-pi's terminal UI; :stop stops the thread's process",
    });
    expect(action("/rewind")).toEqual({
      type: "refuse",
      message: "/branch was not sent: Needs oh-my-pi's terminal UI; omfp:tree navigates the session tree",
    });
    expect(action("/fork pane")).toEqual({
      type: "refuse",
      message: "/fork was not sent: omfp runs it without arguments",
    });
    expect(action("/copy code")).toEqual({
      type: "refuse",
      message: "/copy was not sent: omfp runs it without arguments",
    });
  });

  // spec: UI-978
  test("suggestions: routed builtins say what omfp does, the other TUI-only ones are disabled after the rest", () => {
    const entries = slashEntries(ANNOUNCED);
    const named = (name: string) => entries.find((entry) => entry.name === name);
    expect(named("model")).toEqual({
      name: "model",
      aliases: ["models"],
      description: "Pick the thread's model in the header (:model)",
      hint: undefined,
      disabled: false,
    });
    expect(named("login")).toEqual({
      name: "login",
      aliases: [],
      description: "Log in to a provider (:login)",
      hint: "[<provider>]",
      disabled: false,
    });
    expect(named("setup")).toEqual({
      name: "setup",
      aliases: ["providers"],
      description: "Needs oh-my-pi's terminal UI; :login logs in to a provider",
      hint: undefined,
      disabled: true,
    });
    expect(entries.filter((entry) => entry.name === "copy")).toHaveLength(1);

    const shown = (draft: string) =>
      slashSuggestions(draft, ANNOUNCED).map((suggestion) => `${suggestion.name}${suggestion.disabled ? "!" : ""}`);
    expect(shown("/se")).toEqual(["security", "session", "settings!", "setup!"]);
    expect(shown("/lo")).toEqual(["login", "logout", "loop!", "local!"]);
    expect(shown("/q")).toEqual(["queue!", "quit!"]);
  });

  // spec: UI-978
  test("an announced command shadows a TUI-only builtin of the same name or alias", () => {
    const own: AvailableSlashCommand[] = [
      { name: "settings", source: "file", description: "Project settings" },
      { name: "mine", source: "custom", aliases: ["q"] },
    ];
    const entries = slashEntries(own);
    expect(entries.filter((entry) => entry.name === "settings")).toEqual([
      { name: "settings", aliases: [], description: "Project settings", hint: undefined, disabled: false },
    ]);
    expect(entries.find((entry) => entry.name === "quit")?.aliases).toEqual([]);
  });

  // spec: UI-978
  test("the composer never offers the commands of omfp's own omp extension", () => {
    const entries = slashEntries([
      { name: "omfp-voice-log", source: "extension", description: "omfp: keeps the spoken conversation" },
      { name: "mine", source: "custom" },
    ]);
    expect(entries.some((entry) => entry.name === "omfp-voice-log")).toBe(false);
    expect(entries.some((entry) => entry.name === "mine")).toBe(true);
  });
});
