/**
 * The answer of the card a widget is drawn in (UI-921): an agent's `omfp_card` call (AGT-096) shows a widget in its
 * card in the transcript and, with `wait`, waits until a window answers it with `card.answer` (PRT-224).
 */

import type { CardAnswer } from "@omfp/protocol";
import { createContext, useContext, useMemo } from "react";
import { strings } from "../strings/index.ts";
import type { AnswerState } from "../vm/tool-view.ts";
import { useHost } from "./host.tsx";

/** The card around a widget: its thread's `omfp_card` call and where the call's answer stands. */
export interface CardInfoScope {
  readonly threadId: string;
  readonly toolCallId: string;
  readonly state: AnswerState;
  /** The submitted value once `state` is `submitted`. */
  readonly value: unknown;
}

/** Provided by the transcript around a card's widget and its controls. */
export const CardScope = createContext<CardInfoScope | undefined>(undefined);

/** What {@link useAnswer} gives a widget. */
export interface Answer {
  /** `waiting` while the card's `wait` call runs; `none` without `wait` or outside a card. */
  readonly state: AnswerState;
  /** The submitted value once `state` is `submitted`. */
  readonly value: unknown;
  /** Answers the waiting card with `value` (any JSON), which becomes the tool's result; rejects with the daemon's
   * error, at once unless `state` is `waiting`. */
  submit(value: unknown): Promise<void>;
  /** Answers the waiting card with a cancel; rejects like {@link Answer.submit}. */
  cancel(): Promise<void>;
}

/** The answer of the card this widget is drawn in (UI-921). */
export function useAnswer(): Answer {
  const card = useContext(CardScope);
  const { client } = useHost();
  return useMemo(() => {
    const send = async (answer: CardAnswer): Promise<void> => {
      if (card?.state !== "waiting") {
        throw new Error(strings.widgets.notWaiting);
      }
      await client.call("card.answer", { threadId: card.threadId, toolCallId: card.toolCallId, answer });
    };
    return {
      state: card?.state ?? "none",
      value: card?.value,
      submit: (value) => send({ kind: "submit", value }),
      cancel: () => send({ kind: "cancel" }),
    };
  }, [card, client]);
}
