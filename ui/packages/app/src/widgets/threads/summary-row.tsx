/**
 * One thread of a workspace section (UI-020): the kit's thread row (UI-169, UI-442) for a thread summary, with its
 * unarchive action, the spawned-by filter (UI-240) and a fork's mark naming its source (UI-665); a row that drags
 * carries its place in a drag (UI-621, UI-627).
 */
import { Button, Icon, ThreadRow } from "@omfp/kit";
import type { ThreadSummary } from "@omfp/protocol";
import type { MouseEvent, PointerEventHandler, ReactNode } from "react";
import { useThreadActions, useThreads, useWindowState } from "../../sdk/index.ts";
import { strings } from "../../strings/index.ts";
import { displayTitle, forkLineage, statusLabel, threadMark, threadStatus } from "../../vm/threads.ts";
import { relativeAge } from "./age.ts";
import { SPAWNED_BY } from "./keys.ts";
import { useFresh } from "./marks.ts";

const text = strings.shell.threads;

/** A row that drags (UI-621): the press that may start the drag, and its part in the drag in progress. */
export interface RowReorder {
  readonly onPointerDown: PointerEventHandler;
  /** It is the dragged row: dimmed. */
  readonly dragging: boolean;
  /** The dragged thread lands just before it: the drop line on its top. */
  readonly dropBefore: boolean;
}

export function SummaryRow({
  thread,
  selected,
  unread,
  snippet,
  spawned,
  archived = false,
  reorder,
}: {
  thread: ThreadSummary;
  selected: boolean;
  unread: boolean;
  snippet: string | undefined;
  /** How many threads it spawned, transitively; `undefined` for none (UI-240). */
  spawned: number | undefined;
  archived?: boolean;
  /** Absent: the row does not drag (an archived one). */
  reorder?: RowReorder;
}): ReactNode {
  const actions = useThreadActions();
  const [spawnedBy, setSpawnedBy] = useWindowState(SPAWNED_BY);
  const status = threadStatus(thread);
  const title = displayTitle(thread);
  const fresh = useFresh(status);
  const filtering = spawnedBy === thread.id;
  const lineage = forkLineage(thread, useThreads());
  const middleClick = (event: MouseEvent) => {
    if (event.button === 1) {
      // Prevent autoscroll on press; delete only on release, without selecting the thread.
      event.preventDefault();
      if (event.type === "auxclick") {
        void actions.remove(thread.id);
      }
    }
  };
  return (
    <ThreadRow
      {...(reorder !== undefined && {
        onPointerDown: reorder.onPointerDown,
        "data-reorder-thread": thread.id,
        "data-dragging": reorder.dragging ? "true" : undefined,
        "data-drop": reorder.dropBefore ? "before" : undefined,
      })}
      state={threadMark(status, unread)}
      stateLabel={statusLabel(status)}
      markProps={{ "data-status": status }}
      title={
        lineage === undefined ? (
          title
        ) : (
          <>
            <span className="omfp-threads__forked" title={lineage.label} data-testid="thread-forked">
              <Icon name="git-fork" />
              <span className="omfp-visually-hidden">{lineage.label}</span>
            </span>
            {title}
          </>
        )
      }
      placeholder={thread.titleSource === "placeholder"}
      meta={relativeAge(thread.updatedAt, Date.now())}
      selected={selected}
      fresh={fresh}
      unread={unread}
      unreadLabel={text.unread}
      archived={archived}
      snippet={snippet}
      onSelect={() => actions.select(thread.id)}
      selectProps={{
        "data-testid": "thread-row",
        "data-thread-id": thread.id,
        "data-status": status,
        "data-title-source": thread.titleSource,
        "data-archived": archived ? "true" : undefined,
        "data-reorder-handle": reorder === undefined ? undefined : "true",
        onMouseDown: middleClick,
        onAuxClick: middleClick,
      }}
      actions={
        <>
          {archived && (
            <Button
              size="sm"
              variant="quiet"
              aria-label={text.unarchiveThread(title)}
              data-testid="unarchive-thread"
              data-thread-id={thread.id}
              onPress={() => void actions.unarchive(thread.id)}
            >
              {text.unarchive}
            </Button>
          )}
          {spawned !== undefined && (
            <Button
              size="sm"
              variant="quiet"
              className="omfp-threads__spawned-filter"
              icon={<Icon name="git-fork" />}
              aria-label={text.spawnedFilter(spawned, title)}
              aria-pressed={filtering}
              data-testid="spawned-filter"
              data-thread-id={thread.id}
              onPress={() => setSpawnedBy(filtering ? undefined : thread.id)}
            >
              {spawned}
            </Button>
          )}
        </>
      }
      onDelete={() => void actions.remove(thread.id)}
      deleteLabel={text.deleteThread(title)}
      deleteProps={{ "data-testid": "row-delete-thread", "data-thread-id": thread.id }}
    />
  );
}
