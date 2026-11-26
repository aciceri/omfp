/**
 * Files in a view without a `main` slot (UI-115): a panel floating over the view holding the open files as tabs, like
 * the `main` slot's (selected tab, each tab's own `×`, middle click and Delete, closing selects the neighbour,
 * window-local; UI-365, UI-366). The panel moves by its grip and resizes by its corner (pointer or arrow keys),
 * minimises to its bar, and keeps its place and state in this window. Esc closes the selected file (an Esc the file
 * editor kept for itself does not count). Every file stays mounted, hidden with its effects paused while another tab
 * shows or the panel is minimised, so it shows again as it was, unsaved edits included (UI-401).
 */

import { Tab, TabList, TabPanel, Tabs } from "@omfp/kit";
import type { ViewsState } from "@omfp/protocol";
import { type KeyboardEvent, type ReactNode, useRef } from "react";
import { FLOATING_FILES, fileName, fileTabId, useFileTabs } from "../sdk/index.ts";
import { strings } from "../strings/index.ts";
import { ClosableTabLabel, closableTabBarHandlers } from "../view/closable-tab.tsx";
import { FILE_WIDGET, fileProps } from "../view/layout.ts";
import { useTabSteps } from "../view/render.tsx";
import type { WidgetRegistry } from "../view/resolve.ts";
import { WidgetSlot } from "../view/slot.tsx";
import { floatingControls, useFloatingBox } from "./floating-box.tsx";

const text = strings.shell.files;

export interface FloatingFileProps {
  readonly workspaceId: string | undefined;
  readonly views: ViewsState | undefined;
  readonly registry: WidgetRegistry;
}

export function FloatingFile({ workspaceId, views, registry }: FloatingFileProps): ReactNode {
  const files = useFileTabs(workspaceId);
  const box = useFloatingBox(FLOATING_FILES);
  const { panel, root, style } = box;
  const controls = floatingControls(box, "floating-file", text);
  // `g t` / `g T` from a focused file tab (UI-095): the tab bar sits right in the tabs container.
  const anchor = useRef<HTMLDivElement>(null);
  const ids = files.tabs.map((tab) => fileTabId(tab.path));
  const active = files.active !== undefined && ids.includes(files.active) ? files.active : ids[0];
  useTabSteps(anchor, ids, active, files.select);
  const first = files.tabs[0];
  if (first === undefined) {
    return null;
  }
  const selected = files.tabs.find((tab) => fileTabId(tab.path) === files.active) ?? first;
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape" && !event.defaultPrevented) {
      event.preventDefault();
      files.close(selected.path);
    }
  };
  const bar = (tabs: ReactNode) => (
    <div ref={anchor} className="omfp-floating-file__bar" {...closableTabBarHandlers(files.closeTab)}>
      {controls.grip}
      {tabs}
      {controls.minimise}
    </div>
  );
  return (
    <section
      ref={root}
      className="omfp-floating-file"
      aria-label={text.panel(files.tabs.length)}
      data-testid="floating-file"
      data-minimised={panel.minimised ? "" : undefined}
      style={style}
      onKeyDown={onKeyDown}
    >
      <Tabs
        className="omfp-floating-file__tabs"
        selectedKey={fileTabId(selected.path)}
        onSelectionChange={(key) => files.select(String(key))}
      >
        {bar(
          <>
            {panel.minimised && <span className="omfp-floating-file__title">{text.panel(files.tabs.length)}</span>}
            {/* Minimised, the tab list stays (it holds the tabs the panels belong to), hidden. */}
            <TabList aria-label={text.tabs} className="omfp-floating-file__list" hidden={panel.minimised}>
              {files.tabs.map((tab) => (
                <Tab
                  key={fileTabId(tab.path)}
                  id={fileTabId(tab.path)}
                  data-testid="floating-file-tab"
                  data-file={tab.path}
                  data-tab={fileTabId(tab.path)}
                >
                  <ClosableTabLabel
                    id={fileTabId(tab.path)}
                    title={fileName(tab.path)}
                    tooltip={tab.path}
                    kind="file"
                    onClose={files.closeTab}
                  />
                </Tab>
              ))}
            </TabList>
          </>,
        )}
        {files.tabs.map((tab) => (
          <TabPanel
            key={fileTabId(tab.path)}
            id={fileTabId(tab.path)}
            className="omfp-floating-file__body"
            keepMounted
            hidden={panel.minimised}
          >
            <WidgetSlot
              reference={FILE_WIDGET}
              props={fileProps(tab)}
              path={fileTabId(tab.path)}
              workspaceId={workspaceId}
              views={views}
              registry={registry}
            />
          </TabPanel>
        ))}
      </Tabs>
      {controls.resize}
    </section>
  );
}
