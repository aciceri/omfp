/** Space v selects a view by fuzzy query, arrows and Enter, or its visible 1–9 hint. */
import { useState } from "react";
import { useCommand } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import type { ModeLineViews } from "./mode-line.tsx";
import { Picker } from "./picker.tsx";

export function ViewPicker({ views }: { views: ModeLineViews }) {
  const [session, setSession] = useState<{ focus: Element | null }>();
  useCommand({
    id: "view.pick",
    title: strings.shell.commands.pickView,
    enabled: () => views.views.length > 0,
    run: () => setSession({ focus: document.activeElement }),
  });
  if (session === undefined) return null;
  const close = () => {
    setSession(undefined);
    queueMicrotask(() => {
      if (session.focus instanceof HTMLElement && session.focus.isConnected) session.focus.focus();
    });
  };
  return (
    <Picker
      id="view-picker"
      title={strings.shell.commands.pickView}
      empty={strings.shell.frame.noViewMatch}
      options={views.views.map((view) => ({ ...view, description: view.id }))}
      initial={views.current}
      numbered
      choose={(id) => {
        views.onSelect(id);
        close();
      }}
      cancel={close}
    />
  );
}
