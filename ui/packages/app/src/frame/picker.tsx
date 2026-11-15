/** Keyboard picker: fuzzy subsequence filtering, live highlight, Enter commits, Escape/blur cancels. */
import { CommandItem, CommandLine, CommandMenu, Kbd } from "@omfp/kit";
import { type ReactNode, useEffect, useId, useState } from "react";
import { CMDLINE_ATTRIBUTE } from "../keymap/handler.ts";
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

export function Picker({
  id,
  title,
  empty,
  options,
  initial,
  preview,
  numbered = false,
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
  choose: (id: string) => void;
  cancel: () => void;
}): ReactNode {
  const [value, setValue] = useState("");
  const [active, setActive] = useState(initial);
  const listId = useId();
  const shown = options.filter((option) => matches(`${option.title} ${option.description}`, value));
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
        listProps={{ id: listId, "aria-label": title }}
        footer={
          <>
            <Kbd>up</Kbd>
            <Kbd>down</Kbd>
            {preview === undefined ? strings.shell.frame.pickerKeys.move : strings.shell.frame.pickerKeys.preview}
            <Kbd>enter</Kbd>
            {strings.shell.frame.pickerKeys.choose}
            <Kbd>esc</Kbd>
            {strings.shell.frame.commandKeys.cancel}
            {numbered && (
              <>
                <Kbd>1–9</Kbd>
                {strings.shell.frame.pickerKeys.choose}
              </>
            )}
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
                    <Kbd>{at + 1}</Kbd> {option.description}
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
