/**
 * A closable tab's label and the ways to close it (UI-365, UI-366, UI-373, UI-375), shared by the `main` slot's file
 * and widget tabs, the floating file panel and the closable tabs of a view's layout: each tab carries its own `×`, a
 * middle click on a tab closes it (its press never starts the browser's autoscroll), Delete closes the focused tab.
 * None selects the tab first. An opened tab's title (a file, a revealed widget: this window's, never in `views.json`)
 * is marked `data-opened`, which shows it in italics (UI-377).
 */

import { Icon } from "@omfp/kit";
import type { KeyboardEvent, MouseEvent, ReactNode } from "react";
import { strings } from "../strings/index.ts";

const text = strings.shell.files;

/** The id of the closable tab an event happened in (closable tabs carry `data-tab`). */
function tabOf(event: MouseEvent | KeyboardEvent): string | undefined {
  return (event.target as Element).closest<HTMLElement>("[data-tab]")?.dataset.tab;
}

/** Handlers for the element around a tab list: middle click and Delete close the closable tab they target. */
export function closableTabBarHandlers(close: (id: string) => void) {
  const middle = (event: MouseEvent) => {
    const id = event.button === 1 ? tabOf(event) : undefined;
    if (id !== undefined) {
      event.preventDefault();
      if (event.type === "auxclick") {
        close(id);
      }
    }
  };
  return {
    onMouseDown: middle,
    onAuxClick: middle,
    onKeyDown: (event: KeyboardEvent) => {
      const id = event.key === "Delete" ? tabOf(event) : undefined;
      if (id !== undefined) {
        event.preventDefault();
        close(id);
      }
    },
  };
}

export interface ClosableTabLabelProps {
  /** The tab's id (a file's `fileTabId`, a widget's `widgetTabId`, a layout tab's path), what `onClose` gets. */
  readonly id: string;
  /** What the tab shows: the file's name, the widget's title. */
  readonly title: string;
  /** The label's tooltip: the file's path, the widget's reference. */
  readonly tooltip: string;
  /** An opened `file` or `widget` tab, or a `layout` tab; the close mark's test id is `<kind>-tab-close`. */
  readonly kind: "file" | "widget" | "layout";
  readonly onClose: (id: string) => void;
}

/** The title of a closable tab and its own `×`; the `×` is a pointer target (Delete is the keyboard way). */
export function ClosableTabLabel({ id, title, tooltip, kind, onClose }: ClosableTabLabelProps): ReactNode {
  return (
    <>
      <span title={tooltip} data-opened={kind === "layout" ? undefined : true}>
        {title}
      </span>
      <span
        className="omfp-file-tab__close"
        aria-hidden="true"
        title={text.close(title)}
        data-testid={`${kind}-tab-close`}
        // The tab selects itself on press: the `×` closes without selecting.
        onPointerDown={(event) => event.stopPropagation()}
        onMouseDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onClose(id);
        }}
      >
        <Icon name="x" />
      </span>
    </>
  );
}
