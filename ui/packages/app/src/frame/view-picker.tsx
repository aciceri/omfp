/** Space v selects a view by fuzzy query, arrows and Enter, or its visible 1–9 hint. */
import { strings } from "../strings/index.ts";
import type { ModeLineViews } from "./mode-line.tsx";
import { Picker, usePickerSession } from "./picker.tsx";

export function ViewPicker({ views }: { views: ModeLineViews }) {
  const [session, close] = usePickerSession(
    { id: "view.pick", title: strings.shell.commands.pickView, enabled: () => views.views.length > 0 },
    () => ({}),
  );
  if (session === undefined) return null;
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
