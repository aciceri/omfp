import type { HTMLAttributes } from "react";
import { cx } from "./class.ts";

export type KbdProps = HTMLAttributes<HTMLElement>;

/** A key chip (UI-148): a key or key sequence as lowercase words (`spc n`, `enter`), e.g. in hints and buttons. */
export function Kbd({ className, ...props }: KbdProps) {
  return <kbd {...props} className={cx("omfp-kbd", className)} />;
}

export type KeyHintProps = HTMLAttributes<HTMLSpanElement>;

/**
 * A hint that only teaches keys (UI-418): its key chips with their label (and separator), e.g. `esc cancel`. Hidden
 * whole on touch-only screens, which have no such keys; content chips (help, which-key) use a bare `Kbd`.
 */
export function KeyHint({ className, ...props }: KeyHintProps) {
  return <span {...props} className={cx("omfp-keyhint", className)} />;
}
