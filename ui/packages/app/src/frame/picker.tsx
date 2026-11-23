/** Keyboard picker: fuzzy subsequence filtering, live highlight, Enter commits, Escape/blur cancels. */
import { CommandItem, CommandLine, CommandMenu, Kbd, KeyHint } from "@omfp/kit";
import { type ReactNode, useEffect, useId, useState } from "react";
import { CMDLINE_ATTRIBUTE } from "../keymap/handler.ts";
import { type Command, useCommand } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";

export interface PickerOption {
  readonly id: string;
  readonly title: string;
  readonly description: string;
}

function matches(text: string, query: string): boolean {
  let at = 0;
  const lowered = text.toLowerCase();
  for (const char of query.toLowerCase().replace(/\s/g, "")) {
    at = lowered.indexOf(char, at);
    if (at < 0) return false;
    at += 1;
  }
  return true;
}

/**
 * A picker `command` opens: `open`'s state when it ran plus the element that had the focus, and `close`, which ends the
 * session and gives that element the focus back.
 */
export function usePickerSession<T extends object>(
  command: Omit<Command, "run">,
  open: () => T,
): readonly [(T & { readonly focus: Element | null }) | undefined, () => void] {
  const [session, setSession] = useState<T & { readonly focus: Element | null }>();
  useCommand({ ...command, run: () => setSession({ ...open(), focus: document.activeElement }) });
  const close = () => {
    const focus = session?.focus;
    setSession(undefined);
    queueMicrotask(() => {
      if (focus instanceof HTMLElement && focus.isConnected) focus.focus();
    });
  };
  return [session, close];
}

export function Picker({
  id,
  title,
  empty,
  options,
  initial,
  preview,
  numbered = false,
  prompt,
  search,
  notice,
  busy = false,
  choose,
  cancel,
}: {
  id: string;
  title: string;
  empty: string;
  options: readonly PickerOption[];
  initial?: string | undefined;
  preview?: ((id: string) => void) | undefined;
  numbered?: boolean;
  /** Shown before the query, e.g. what the picker is for. */
  prompt?: ReactNode;
  /** An external search supplies already-filtered options, avoiding a second, differently ordered text match. */
  search?: ((query: string) => void) | undefined;
  notice?: string | undefined;
  busy?: boolean;
  choose: (id: string) => void;
  cancel: () => void;
}): ReactNode {
  const [value, setValue] = useState("");
  const [active, setActive] = useState(initial);
  const listId = useId();
  const shown =
    search === undefined
      ? options.filter((option) => matches(`${option.title} ${option.description}`, value))
      : options;
  const highlighted = shown.find((option) => option.id === active) ?? shown[0];
  const highlightedId = highlighted?.id;
  useEffect(() => {
    if (highlightedId !== undefined) {
      preview?.(highlightedId);
      document.getElementById(`${listId}-${highlightedId}`)?.scrollIntoView?.({ block: "nearest" });
    }
  }, [highlightedId, listId, preview]);
  return (
    <div className="omfp-cmdline" data-testid={id}>
      <CommandMenu
        className="omfp-cmdline__menu"
        title={title}
        listProps={{ id: listId, "aria-label": title, "aria-busy": busy }}
        footer={
          <>
            <KeyHint>
              <Kbd>up</Kbd>
              <Kbd>down</Kbd>
              {preview === undefined ? strings.shell.frame.pickerKeys.move : strings.shell.frame.pickerKeys.preview}
            </KeyHint>
            <KeyHint>
              <Kbd>enter</Kbd>
              {strings.shell.frame.pickerKeys.choose}
            </KeyHint>
            <KeyHint>
              <Kbd>esc</Kbd>
              {strings.shell.frame.commandKeys.cancel}
            </KeyHint>
            {numbered && (
              <KeyHint>
                <Kbd>1–9</Kbd>
                {strings.shell.frame.pickerKeys.choose}
              </KeyHint>
            )}
            {notice !== undefined && <span role="status">{notice}</span>}
          </>
        }
      >
        {shown.length === 0 ? (
          <li className="omfp-muted">{empty}</li>
        ) : (
          shown.map((option, at) => (
            <CommandItem
              key={option.id}
              id={`${listId}-${option.id}`}
              command={option.title}
              description={
                numbered && at < 9 ? (
                  <>
                    <KeyHint>
                      <Kbd>{at + 1}</Kbd>
                    </KeyHint>{" "}
                    {option.description}
                  </>
                ) : (
                  option.description
                )
              }
              selected={option.id === highlighted?.id}
              data-testid={`${id}-option`}
              data-id={option.id}
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => choose(option.id)}
            />
          ))
        )}
      </CommandMenu>
      <CommandLine>
        {prompt}
        <input
          className="omfp-cmd__input"
          data-testid={`${id}-input`}
          {...{ [CMDLINE_ATTRIBUTE]: "" }}
          role="combobox"
          aria-label={title}
          aria-expanded={true}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={highlighted === undefined ? undefined : `${listId}-${highlighted.id}`}
          autoComplete="off"
          spellCheck={false}
          // biome-ignore lint/a11y/noAutofocus: opening a picker deliberately transfers focus to its query.
          autoFocus
          value={value}
          onChange={(event) => {
            setValue(event.target.value);
            setActive(undefined);
            search?.(event.target.value);
          }}
          onBlur={cancel}
          onKeyDown={(event) => {
            if (event.key === "Escape") cancel();
            else if (numbered && /^[1-9]$/.test(event.key) && !event.ctrlKey && !event.altKey && !event.metaKey) {
              const option = shown[Number(event.key) - 1];
              if (option !== undefined) choose(option.id);
            } else if (event.key === "Enter") {
              if (highlighted !== undefined) choose(highlighted.id);
            } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              const at = shown.findIndex((option) => option.id === highlighted?.id);
              setActive(shown[(at + (event.key === "ArrowDown" ? 1 : -1) + shown.length) % shown.length]?.id);
            } else return;
            event.preventDefault();
            event.stopPropagation();
          }}
        />
      </CommandLine>
    </div>
  );
}
