/**
 * The `:` command line (UI-095), also the command palette: an input with a listbox of completions (ARIA combobox
 * pattern). Tab completes, the arrows move through the options, Enter runs the line (or the highlighted option),
 * Escape closes. Commands run in the scopes around the element that had the focus before the line opened, and the
 * focus goes back there first. `cmdline.open` with arguments opens the line on them, followed by a space, so a
 * command with arguments can immediately offer its completions.
 */

import { CommandItem, CommandMenu, Kbd, KeyHint, ModeSegment, CommandLine as Prompt } from "@omfp/kit";
import { type KeyboardEvent, type ReactNode, useEffect, useId, useReducer, useRef, useState } from "react";
import { CMDLINE_ATTRIBUTE } from "../keymap/handler.ts";
import { type Binding, keyLabel, keysFor } from "../keymap/keymap.ts";
import { scopesAt, useCommand, useHost } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { type Completion, completions, onlyScope, parseCommandLine } from "./cmdline.ts";

const text = strings.shell.frame;

export function CommandLine({ bindings }: { bindings: readonly Binding[] }): ReactNode {
  // The line's first text while open; `undefined` while closed.
  const [open, setOpen] = useState<string | undefined>(undefined);
  const previous = useRef<Element | null>(null);
  useCommand({
    id: "cmdline.open",
    title: strings.shell.commands.cmdline,
    run: (args) => {
      previous.current = document.activeElement;
      setOpen(args.length === 0 ? "" : `${args.join(" ")} `);
    },
  });
  if (open === undefined) {
    return null;
  }
  /** Closes the line; `refocus` puts the focus back where it was (not when the user clicked elsewhere). */
  const close = (refocus: boolean) => {
    setOpen(undefined);
    const back = previous.current;
    if (refocus && back instanceof HTMLElement && back.isConnected) {
      back.focus();
    }
  };
  return (
    <CommandLineInput initial={open} bindings={bindings} close={close} scopes={() => scopesAt(previous.current)} />
  );
}

function CommandLineInput({
  initial,
  bindings,
  close,
  scopes,
}: {
  initial: string;
  bindings: readonly Binding[];
  close: (refocus: boolean) => void;
  scopes: () => string[];
}): ReactNode {
  const { commands } = useHost();
  const [value, setValue] = useState(initial);
  const [active, setActive] = useState<number | undefined>(undefined);
  const [error, setError] = useState<string | undefined>(undefined);
  const listId = useId();
  // Completions follow commands registered or removed while the line is open.
  const [, changed] = useReducer((count: number) => count + 1, 0);
  useEffect(() => commands.subscribe(changed), [commands]);
  const options = completions(value, commands.list(), scopes());
  const highlighted = active === undefined ? undefined : options[active];
  // The arrows move the highlight past the menu's visible rows: keep the highlighted option in view (UI-336).
  useEffect(() => {
    if (active !== undefined) {
      document.getElementById(`${listId}-${active}`)?.scrollIntoView({ block: "nearest" });
    }
  }, [active, listId]);
  const run = (line: string) => {
    const parsed = parseCommandLine(line);
    if (parsed === undefined) {
      close(true);
      return;
    }
    const where = scopes();
    if (commands.resolve(parsed.name, where) === undefined) {
      const scope = onlyScope(parsed.name, commands.list());
      setError(scope === undefined ? text.unknownCommand(parsed.name) : text.scopedCommand(parsed.name, scope));
      return;
    }
    close(true);
    commands.run(parsed.name, parsed.args, where);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const handled = handleKey(event.key, event.shiftKey);
    if (handled) {
      event.preventDefault();
      event.stopPropagation();
    }
  };
  const handleKey = (key: string, shift: boolean): boolean => {
    switch (key) {
      case "Escape":
        close(true);
        return true;
      case "Enter":
        run(highlighted?.value ?? lineToRun(value, options));
        return true;
      case "Tab": {
        const choice = highlighted ?? options[0];
        if (choice !== undefined && !shift) {
          setValue(`${choice.value} `);
          setActive(undefined);
        }
        return true;
      }
      case "ArrowDown":
      case "ArrowUp":
        setActive(step(active, options.length, key === "ArrowDown" ? 1 : -1));
        return true;
      default:
        return false;
    }
  };
  const typed = value.trimStart().toLowerCase();
  return (
    <div className="omfp-cmdline" data-testid="command-line">
      <CommandMenu
        className="omfp-cmdline__menu"
        title={text.commandMenu}
        hidden={options.length === 0}
        listProps={{ id: listId, "aria-label": text.commandLine }}
        footer={<MenuKeys />}
      >
        {options.map((option, index) => (
          <CommandOption
            key={option.value}
            id={`${listId}-${index}`}
            option={option}
            matched={option.value.toLowerCase().startsWith(typed) ? typed.length : 0}
            selected={index === active}
            keys={keysFor(bindings, option.command)}
            onChoose={() => run(option.value)}
          />
        ))}
      </CommandMenu>
      <Prompt>
        <input
          className="omfp-cmd__input"
          data-testid="command-line-input"
          {...{ [CMDLINE_ATTRIBUTE]: "" }}
          role="combobox"
          aria-label={text.commandLine}
          aria-expanded={options.length > 0}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={highlighted === undefined ? undefined : `${listId}-${active}`}
          autoComplete="off"
          spellCheck={false}
          // biome-ignore lint/a11y/noAutofocus: the line opens on `:` and takes the focus
          autoFocus
          // The caret after the text the line opened on.
          onFocus={(event) => event.currentTarget.setSelectionRange(value.length, value.length)}
          value={value}
          onChange={(event) => {
            setValue(event.target.value);
            setActive(undefined);
            setError(undefined);
          }}
          onKeyDown={onKeyDown}
          onBlur={() => close(false)}
        />
      </Prompt>
      {error !== undefined && (
        <ModeSegment kind="error" role="alert" data-testid="command-line-error">
          {error}
        </ModeSegment>
      )}
    </div>
  );
}

/** The key row of the completion menu: `tab` complete, `enter` run, `esc` cancel (UI-168). */
export function MenuKeys(): ReactNode {
  return (
    <>
      <KeyHint>
        <Kbd>{keyLabel("Tab")}</Kbd>
        <span>{text.commandKeys.complete}</span>
      </KeyHint>
      <KeyHint>
        <Kbd>{keyLabel("Enter")}</Kbd>
        <span>{text.commandKeys.run}</span>
      </KeyHint>
      <KeyHint>
        <Kbd>{keyLabel("Escape")}</Kbd>
        <span>{text.commandKeys.cancel}</span>
      </KeyHint>
    </>
  );
}

/** The typed line when it is a completion itself, else the first completion it starts (`arch` → `archive`). */
function lineToRun(value: string, options: readonly Completion[]): string {
  const typed = value.trim();
  if (options.some((option) => option.value === typed)) {
    return typed;
  }
  return options.find((option) => option.value.startsWith(typed))?.value ?? typed;
}

function step(active: number | undefined, count: number, delta: number): number | undefined {
  if (count === 0) {
    return undefined;
  }
  if (active === undefined) {
    return delta > 0 ? 0 : count - 1;
  }
  return (active + delta + count) % count;
}

function CommandOption({
  id,
  option,
  matched,
  selected,
  keys,
  onChoose,
}: {
  id: string;
  option: Completion;
  matched: number;
  selected: boolean;
  keys: readonly string[];
  onChoose: () => void;
}): ReactNode {
  return (
    <CommandItem
      id={id}
      command={option.value}
      matched={matched}
      selected={selected}
      description={
        <>
          <span className="omfp-cmdline__title">{option.title}</span>
          {keys.map((key) => (
            <Kbd key={key}>{keyLabel(key)}</Kbd>
          ))}
        </>
      }
      data-testid="command-option"
      data-value={option.value}
      // Keeps the focus in the input, which would otherwise blur and close the line before the click.
      onMouseDown={(event) => event.preventDefault()}
      onClick={onChoose}
    />
  );
}
