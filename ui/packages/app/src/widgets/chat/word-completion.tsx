/**
 * omp's word completion in the composer (UI-175..UI-179), shared by the thread's composer and the draft thread's: after
 * each edit leaving the caret at the end of a word, `prompts.predict_word` gives the rest of the word, drawn dimmed
 * after the caret by a copy of the text area's text laid over it; Tab or ArrowRight insert it, typing past it refuses
 * it (`prompts.predict_word_feedback`). Neither call ever shows an error.
 */

import { acceptGhost, followGhost, useCall, type WordGhost, wordAt, wordGhost } from "omfp/sdk";
import { type RefObject, type SyntheticEvent, useEffect, useLayoutEffect, useRef, useState } from "react";
import { replace } from "./line-keys.ts";

/** The text and the collapsed caret an edit left. */
interface Cursor {
  readonly text: string;
  readonly caret: number;
}

/** The word completion of a composer: see {@link useWordCompletion}. */
export interface WordCompletion {
  /** The ghost to draw after the caret, if one shows. */
  readonly ghost: WordGhost | undefined;
  /** After each edit of the text (typing, paste, deletion): narrows or ends the ghost, asks for the new text. */
  typed(): void;
  /** The form's `onSelect`: a caret move or a selection ends the ghost (refused). */
  onSelect(event: SyntheticEvent): void;
  /** Ends the ghost without feedback: blur, send, a prompt-history recall. */
  dismiss(): void;
  /**
   * Inserts the shown ghost's suffix (then a space with `space`) at the caret as typing would, so the text area can
   * undo it (`apply` gets the text where the page cannot), the caret after it, and reports it accepted; answers whether
   * a ghost showed.
   */
  accept(space: boolean, apply: (text: string) => void): boolean;
}

/**
 * The word completion of the composer whose field is `inputRef` and whose text is `value` (UI-175..UI-178); `enabled`
 * is false on a touch screen and while a `/` or `@` suggestion list is open: nothing is asked and no ghost shows.
 */
export function useWordCompletion({
  inputRef,
  value,
  enabled,
}: {
  inputRef: RefObject<HTMLTextAreaElement | null>;
  value: string;
  enabled: boolean;
}): WordCompletion {
  const call = useCall();
  /** The text and caret of the last edit: asked about while it is current. */
  const [asked, setAsked] = useState<Cursor>();
  const [answer, setAnswer] = useState<WordGhost>();
  /** The text and caret the last edit left, until the caret moves: a selection event there is no move. */
  const last = useRef<Cursor | undefined>(undefined);
  const ghost = enabled && answer?.text === value ? answer : undefined;
  /** The ghost on screen, for the event handlers. */
  const shown = useRef(ghost);
  shown.current = ghost;

  useEffect(() => {
    if (!enabled || asked === undefined || wordAt(asked.text, asked.caret) === undefined) {
      return undefined;
    }
    // An answer for an older text or caret is dropped: this effect is cleaned up once they change.
    let live = true;
    call("prompts.predict_word", { text: asked.text, cursor: asked.caret }).then(
      (result) => {
        if (live) {
          setAnswer(wordGhost(asked.text, asked.caret, result.suffix));
        }
      },
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [call, enabled, asked]);

  const feedback = (shownGhost: WordGhost, accepted: boolean) => {
    call("prompts.predict_word_feedback", {
      text: shownGhost.text,
      cursor: shownGhost.cursor,
      suggestion: shownGhost.suffix,
      accepted,
    }).catch(() => undefined);
  };
  const clear = () => {
    last.current = undefined;
    setAnswer(undefined);
    setAsked(undefined);
  };
  /** An accept's own insertion is under way: the `input` event it fires is no edit of the user's. */
  const accepting = useRef(false);

  return {
    ghost,
    typed: () => {
      const input = inputRef.current;
      if (input === null || accepting.current) {
        return;
      }
      const { value: text, selectionStart, selectionEnd } = input;
      const collapsed = selectionStart === selectionEnd;
      const showing = shown.current;
      const change = showing === undefined || !collapsed ? undefined : followGhost(showing, text, selectionEnd);
      if (showing !== undefined && change?.type !== "keep" && change?.type !== "typed") {
        feedback(showing, false);
      }
      setAnswer(change?.type === "keep" ? change.ghost : undefined);
      const at = collapsed ? { text, caret: selectionEnd } : undefined;
      last.current = at;
      setAsked((previous) => (previous?.text === at?.text && previous?.caret === at?.caret ? previous : at));
    },
    onSelect: (event) => {
      const input = inputRef.current;
      const before = last.current;
      // Before the edit's own handling, or after a recall or an accept, the text is not the last edit's: no move.
      if (event.target !== input || input === null || before === undefined || input.value !== before.text) {
        return;
      }
      if (input.selectionStart === before.caret && input.selectionEnd === before.caret) {
        return;
      }
      if (shown.current !== undefined) {
        feedback(shown.current, false);
      }
      clear();
    },
    dismiss: clear,
    accept: (space, apply) => {
      const showing = shown.current;
      const input = inputRef.current;
      if (showing === undefined || input === null) {
        return false;
      }
      // Inserted as typing would, so the text area can undo it; its `input` event neither refuses the ghost nor asks
      // for the completed word.
      const next = acceptGhost(showing, space);
      clear();
      accepting.current = true;
      try {
        replace(
          input,
          {
            kind: "edit",
            from: showing.cursor,
            to: showing.cursor,
            insert: next.text.slice(showing.cursor, next.caret),
            caret: next.caret,
          },
          apply,
        );
      } finally {
        accepting.current = false;
      }
      feedback(showing, true);
      return true;
    },
  };
}

/** The text area's styles its copy takes, so its text wraps and lines up as the text area's does (UI-179). */
const MIRRORED = [
  "direction",
  "fontFamily",
  "fontFeatureSettings",
  "fontKerning",
  "fontSize",
  "fontStretch",
  "fontStyle",
  "fontVariant",
  "fontVariationSettings",
  "fontWeight",
  "letterSpacing",
  "lineHeight",
  "paddingBlockEnd",
  "paddingBlockStart",
  "paddingInlineEnd",
  "paddingInlineStart",
  "tabSize",
  "textIndent",
  "textTransform",
  "wordSpacing",
] as const;

/**
 * The ghost text (UI-175, UI-179): a copy of the text area's text, transparent, laid over the text
 * area's padding box with its font, padding and scroll, so it wraps as the text area's text does; the dimmed suffix
 * hangs out of the flow from a zero-width anchor at the caret, so it never moves that text: a suffix too long for the
 * line is cut at the right edge rather than wrapped. Hidden from assistive technology; the pointer goes through it.
 * Mount it next to the text area, inside an element positioned for it (`omfp-compose__ghost-host`).
 */
export function GhostText({ inputRef, ghost }: { inputRef: RefObject<HTMLTextAreaElement | null>; ghost: WordGhost }) {
  const ref = useRef<HTMLDivElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new ghost is new text to lay over the text area again
  useLayoutEffect(() => {
    const input = inputRef.current;
    const mirror = ref.current;
    const host = mirror?.parentElement;
    if (input === null || mirror === null || host === null || host === undefined) {
      return undefined;
    }
    const computed = getComputedStyle(input);
    for (const name of MIRRORED) {
      mirror.style[name] = computed[name];
    }
    const place = () => {
      const box = input.getBoundingClientRect();
      const origin = host.getBoundingClientRect();
      mirror.style.top = `${box.top - origin.top + input.clientTop}px`;
      mirror.style.left = `${box.left - origin.left + input.clientLeft}px`;
      mirror.style.width = `${input.clientWidth}px`;
      mirror.style.height = `${input.clientHeight}px`;
      mirror.scrollTop = input.scrollTop;
      mirror.scrollLeft = input.scrollLeft;
    };
    place();
    input.addEventListener("scroll", place);
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(place) : undefined;
    observer?.observe(input);
    return () => {
      input.removeEventListener("scroll", place);
      observer?.disconnect();
    };
  }, [inputRef, ghost]);
  return (
    <div ref={ref} className="omfp-compose__ghost" aria-hidden="true" data-testid="composer-ghost-text">
      {ghost.text.slice(0, ghost.cursor)}
      <span className="omfp-compose__ghost-anchor">
        <span className="omfp-compose__ghost-suffix" data-testid="composer-ghost">
          {ghost.suffix}
        </span>
      </span>
      {/* The zero-width space keeps a last empty line, as the text area shows one after a final line break. */}
      {`${ghost.text.slice(ghost.cursor)}\u200b`}
    </div>
  );
}
