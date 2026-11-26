/**
 * The help panel (UI-198): `:help` or `?` (`help.open`) opens a modal listing every effective binding and `:` command
 * (frame/help.ts), live: rebinding in `[keymap]` or a widget registering a command updates it while open. Typing in
 * its filter narrows the rows; Escape, a click outside or the key that opened it (while the filter is empty) closes
 * it, and the focus goes back where it was.
 */

import { HelpDialog, HelpRow, HelpSection, Icon, Kbd, TextField } from "@omfp/kit";
import { type KeyboardEvent, type ReactNode, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { type Binding, KeymapMatcher, keyLabel, keysFor, keyToken } from "../keymap/keymap.ts";
import { useCommand, useHost } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { filterHelp, helpSections } from "./help.ts";

const text = strings.shell.help;

export function Help({ bindings, now }: { bindings: readonly Binding[]; now: () => number }): ReactNode {
  const [open, setOpen] = useState(false);
  const previous = useRef<Element | null>(null);
  const restore = useRef(false);
  useCommand({
    id: "help.open",
    title: strings.shell.commands.help,
    aliases: ["help"],
    run: () => {
      previous.current = document.activeElement;
      setOpen(true);
    },
  });
  // After the modal is gone, so its focus containment does not pull the focus back in.
  useEffect(() => {
    const back = previous.current;
    if (!open && restore.current) {
      restore.current = false;
      if (back instanceof HTMLElement && back.isConnected) {
        back.focus();
      }
    }
  }, [open]);
  if (!open) {
    return null;
  }
  const close = () => {
    restore.current = true;
    setOpen(false);
  };
  return <HelpPanel bindings={bindings} now={now} close={close} />;
}

function HelpPanel({
  bindings,
  now,
  close,
}: {
  bindings: readonly Binding[];
  now: () => number;
  close: () => void;
}): ReactNode {
  const { commands } = useHost();
  // Commands registered or removed while the panel is open show at once.
  const [, changed] = useReducer((count: number) => count + 1, 0);
  useEffect(() => commands.subscribe(changed), [commands]);
  const [query, setQuery] = useState("");
  const sections = filterHelp(helpSections(bindings, commands.list()), query);
  // The keys of `help.open` in any mode close the panel again.
  const closer = useMemo(
    () =>
      new KeymapMatcher(
        bindings
          .filter((binding) => binding.command === "help.open")
          .map((binding) => ({ ...binding, mode: "view" as const })),
        now,
      ),
    [bindings, now],
  );
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const token = keyToken(event.nativeEvent);
    if (token === undefined || event.nativeEvent.isComposing) {
      return;
    }
    // A character typed into a filter that already has text is part of the filter.
    if (query !== "" && ([...token].length === 1 || token === "Space")) {
      return;
    }
    const match = closer.feed(token, "view");
    if (match.kind === "none") {
      return;
    }
    event.preventDefault();
    if (match.kind === "command") {
      close();
    }
  };
  return (
    <HelpDialog
      title={text.title}
      label={text.label}
      data-testid="help"
      onClose={close}
      onKeyDown={onKeyDown}
      end={
        <>
          {[...keysFor(bindings, "help.open"), "Escape"].map((keys) => (
            <Kbd key={keys}>{keyLabel(keys)}</Kbd>
          ))}
          <span>{text.close}</span>
        </>
      }
      filter={
        <TextField
          label={text.filter}
          hideLabel
          mono
          icon={<Icon name="search" />}
          placeholder={text.filterPlaceholder}
          value={query}
          onChange={setQuery}
          autoFocus
          inputProps={{ "data-testid": "help-filter", autoComplete: "off", spellCheck: false }}
        />
      }
    >
      {sections.length === 0 ? (
        <p className="omfp-help__empty" data-testid="help-empty">
          {text.noMatch}
        </p>
      ) : (
        sections.map((section) => (
          <HelpSection key={section.id} title={section.title} note={section.note} data-section={section.id}>
            {section.rows.map((row) => (
              <HelpRow
                key={`${row.command ?? ""}\u0000${row.keys.join(" ")}`}
                keys={row.keys}
                args={row.args}
                description={row.description}
                data-testid="help-row"
                data-command={row.command}
              />
            ))}
          </HelpSection>
        ))
      )}
    </HelpDialog>
  );
}
