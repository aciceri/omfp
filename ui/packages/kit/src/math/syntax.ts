/**
 * TeX math in Markdown (UI-850, UI-851): a remark plugin adding two micromark constructs and the mdast nodes they make.
 *
 * - In text, `$…$` is inline math and `$$…$$` display math (also across lines of a paragraph). A single `$` follows
 *   Pandoc: it opens only before a non-space and closes only after a non-space and not before a digit, so prices stay
 *   prose. Neither kind holds another `$`: one ends the attempt, unless a backslash keeps it in the formula (`\$`).
 *   Escaped (`\$`), in a code span or right after another `$`, a `$` opens nothing.
 * - A block opens with a line of two or more `$` and nothing else and closes with a line of at least as many, like a
 *   code fence (`$$x$$` alone on its line is text math).
 *
 * Text math becomes `<code class="language-math math-inline">` (or `math-display`); a block becomes a `<pre>` around
 * `<code class="language-math">` like a ```` ```math ```` fence, its code marked `data-fence-closed` once its closing
 * line is parsed. The block construct follows `micromark-extension-math`'s, whose text rules differ (any spacing).
 */

/** A character code as micromark passes it: negative for line endings, tabs and virtual spaces, `null` at the end. */
type Code = number | null;
type State = (code: Code) => State | undefined;

interface Effects {
  enter(type: string): unknown;
  exit(type: string): unknown;
  consume(code: Code): undefined;
  attempt(construct: Construct, ok: State, nok?: State): State;
}

interface Token {
  readonly type: string;
}

/** A micromark event: `enter` or `exit`, its token, and the context that serializes the token. */
type MicromarkEvent = readonly [string, Token, { sliceSerialize(token: Token, expandTabs?: boolean): string }];

interface TokenizeContext {
  readonly events: readonly MicromarkEvent[];
  readonly interrupt?: boolean | undefined;
  readonly parser: {
    readonly lazy: Readonly<Record<number, boolean | undefined>>;
    readonly constructs: { readonly disable: { readonly null?: readonly string[] | undefined } };
  };
  now(): { readonly line: number };
}

interface Construct {
  readonly name?: string;
  readonly tokenize: (this: TokenizeContext, effects: Effects, ok: State, nok: State) => State;
  readonly previous?: (this: TokenizeContext, code: Code) => boolean;
  readonly concrete?: boolean;
  readonly partial?: boolean;
}

const DOLLAR = 36;
const BACKSLASH = 92;

function lineEnding(code: Code): boolean {
  return code !== null && code < -2;
}

/** A space, a tab or a virtual space. */
function space(code: Code): boolean {
  return code === -2 || code === -1 || code === 32;
}

/** Optional spaces and tabs as a `type` token, at most `max - 1` of them. */
function spaces(effects: Effects, ok: State, type: string, max?: number): State {
  const limit = max === undefined ? Number.POSITIVE_INFINITY : max - 1;
  let size = 0;

  function start(code: Code): State | undefined {
    if (!space(code)) return ok(code);
    effects.enter(type);
    return inside(code);
  }

  function inside(code: Code): State | undefined {
    if (space(code) && size < limit) {
      size += 1;
      effects.consume(code);
      return inside;
    }
    effects.exit(type);
    return ok(code);
  }

  return start;
}

function tokenizeMathText(effects: Effects, ok: State, nok: State): State {
  let size = 0;
  let closing = 0;
  let last: Code = null;

  function start(code: Code): State | undefined {
    effects.enter("omfpMathText");
    effects.enter("omfpMathTextSequence");
    return open(code);
  }

  function open(code: Code): State | undefined {
    if (code === DOLLAR && size < 2) {
      effects.consume(code);
      size += 1;
      return open;
    }
    effects.exit("omfpMathTextSequence");
    // A third `$`, the end of the text, or a single `$` before a space opens nothing.
    if (code === DOLLAR || code === null || (size === 1 && (space(code) || lineEnding(code)))) return nok(code);
    return between(code);
  }

  function between(code: Code): State | undefined {
    if (code === null) return nok(code);
    if (lineEnding(code)) {
      effects.enter("lineEnding");
      effects.consume(code);
      effects.exit("lineEnding");
      last = code;
      return between;
    }
    if (code === DOLLAR) {
      // A single `$` after a space closes nothing, and no `$` may sit inside the formula.
      if (size === 1 && (space(last) || lineEnding(last))) return nok(code);
      effects.enter("omfpMathTextSequence");
      return close(code);
    }
    effects.enter("omfpMathTextData");
    return data(code);
  }

  function data(code: Code): State | undefined {
    if (code === null || code === DOLLAR || lineEnding(code)) {
      effects.exit("omfpMathTextData");
      return between(code);
    }
    effects.consume(code);
    last = code;
    return code === BACKSLASH ? escaped : data;
  }

  /** After a backslash, the next character belongs to the formula, a `$` too. */
  function escaped(code: Code): State | undefined {
    if (code === null || lineEnding(code)) return data(code);
    effects.consume(code);
    last = code;
    return data;
  }

  function close(code: Code): State | undefined {
    if (code === DOLLAR) {
      effects.consume(code);
      closing += 1;
      return close;
    }
    // As many `$` as opened; a single one not before a digit (`$5-$10`).
    if (closing !== size || (size === 1 && code !== null && code >= 48 && code <= 57)) return nok(code);
    effects.exit("omfpMathTextSequence");
    effects.exit("omfpMathText");
    return ok(code);
  }

  return start;
}

const mathText: Construct = {
  name: "omfpMathText",
  tokenize: tokenizeMathText,
  // Not right after another `$`, unless that one was escaped.
  previous(code) {
    return code !== DOLLAR || this.events.at(-1)?.[1].type === "characterEscape";
  },
};

/** A line ending, then the next line unless it is a lazy continuation of a container (it ends the block). */
const nonLazyContinuation: Construct = {
  partial: true,
  tokenize(effects, ok, nok) {
    const lineStart = (code: Code): State | undefined => {
      if (this.parser.lazy[this.now().line]) return nok(code);
      return ok(code);
    };
    return (code) => {
      if (code === null) return ok(code);
      effects.enter("lineEnding");
      effects.consume(code);
      effects.exit("lineEnding");
      return lineStart;
    };
  },
};

function tokenizeMathFlow(this: TokenizeContext, effects: Effects, ok: State, nok: State): State {
  const tail = this.events.at(-1);
  const indent = tail?.[1].type === "linePrefix" ? tail[2].sliceSerialize(tail[1], true).length : 0;
  const closingIndent = this.parser.constructs.disable.null?.includes("codeIndented") ? undefined : 4;
  const interrupt = this.interrupt === true;
  const closingFence: Construct = { partial: true, tokenize: tokenizeClosingFence };
  let size = 0;

  function start(code: Code): State | undefined {
    effects.enter("omfpMathFlow");
    effects.enter("omfpMathFlowFence");
    effects.enter("omfpMathFlowFenceSequence");
    return open(code);
  }

  function open(code: Code): State | undefined {
    if (code === DOLLAR) {
      effects.consume(code);
      size += 1;
      return open;
    }
    if (size < 2) return nok(code);
    effects.exit("omfpMathFlowFenceSequence");
    return spaces(effects, openEnd, "whitespace")(code);
  }

  /** Nothing may follow the opening sequence on its line. */
  function openEnd(code: Code): State | undefined {
    if (code !== null && !lineEnding(code)) return nok(code);
    effects.exit("omfpMathFlowFence");
    if (interrupt) return ok(code);
    return effects.attempt(nonLazyContinuation, lineStart, after)(code);
  }

  /** At a line inside the block: its closing fence, or a line of the formula without the block's indent. */
  function lineStart(code: Code): State | undefined {
    const line = indent > 0 ? spaces(effects, beforeContent, "linePrefix", indent + 1) : beforeContent;
    return effects.attempt(closingFence, after, line)(code);
  }

  function beforeContent(code: Code): State | undefined {
    if (code === null) return after(code);
    if (lineEnding(code)) return effects.attempt(nonLazyContinuation, lineStart, after)(code);
    effects.enter("omfpMathFlowValue");
    return content(code);
  }

  function content(code: Code): State | undefined {
    if (code === null || lineEnding(code)) {
      effects.exit("omfpMathFlowValue");
      return beforeContent(code);
    }
    effects.consume(code);
    return content;
  }

  function after(code: Code): State | undefined {
    effects.exit("omfpMathFlow");
    return ok(code);
  }

  /** A closing fence: up to three spaces (any number without indented code), as many `$` or more, spaces. */
  function tokenizeClosingFence(fence: Effects, closed: State, notClosed: State): State {
    let closing = 0;

    function fenceStart(code: Code): State | undefined {
      fence.enter("omfpMathFlowFenceClose");
      fence.enter("omfpMathFlowFenceSequence");
      return sequence(code);
    }

    function sequence(code: Code): State | undefined {
      if (code === DOLLAR) {
        fence.consume(code);
        closing += 1;
        return sequence;
      }
      if (closing < size) return notClosed(code);
      fence.exit("omfpMathFlowFenceSequence");
      return spaces(fence, end, "whitespace")(code);
    }

    function end(code: Code): State | undefined {
      if (code !== null && !lineEnding(code)) return notClosed(code);
      fence.exit("omfpMathFlowFenceClose");
      return closed(code);
    }

    return spaces(fence, fenceStart, "linePrefix", closingIndent);
  }

  return start;
}

const mathFlow: Construct = { name: "omfpMathFlow", tokenize: tokenizeMathFlow, concrete: true };

interface HastText {
  readonly type: "text";
  readonly value: string;
}

interface HastCode {
  readonly type: "element";
  readonly tagName: "code";
  readonly properties: Readonly<Record<string, unknown>>;
  readonly children: readonly HastText[];
}

/** The mdast nodes made here; `data.h*` is what `mdast-util-to-hast` turns them into. */
interface MathNode {
  readonly type: string;
  value?: string;
  data?: {
    hName: string;
    hProperties: Record<string, unknown>;
    hChildren: readonly (HastText | HastCode)[];
    closed?: boolean;
  };
}

type Handle = (this: CompileContext, token: Token) => void;

/** The part of `mdast-util-from-markdown`'s compile context the handlers use. */
interface CompileContext {
  readonly stack: readonly MathNode[];
  readonly config: {
    readonly enter: Readonly<Record<string, Handle>>;
    readonly exit: Readonly<Record<string, Handle>>;
  };
  enter(node: MathNode, token: Token): void;
  exit(token: Token): void;
  buffer(): void;
  resume(): string;
  sliceSerialize(token: Token): string;
}

const EDGE_LINE_ENDINGS = /^(?:\r?\n|\r)|(?:\r?\n|\r)$/g;

const fromMarkdown = {
  enter: {
    omfpMathText(this: CompileContext, token: Token): void {
      this.enter({ type: "inlineMath", value: "", data: { hName: "code", hProperties: {}, hChildren: [] } }, token);
    },
    omfpMathFlow(this: CompileContext, token: Token): void {
      this.enter({ type: "math", value: "", data: { hName: "pre", hProperties: {}, hChildren: [] } }, token);
    },
    omfpMathFlowFenceClose(this: CompileContext): void {
      const node = this.stack.findLast((candidate) => candidate.type === "math");
      if (node?.data !== undefined) node.data.closed = true;
    },
  },
  exit: {
    omfpMathText(this: CompileContext, token: Token): void {
      const source = this.sliceSerialize(token);
      const size = source.startsWith("$$") ? 2 : 1;
      const value = source.slice(size, -size);
      const node = this.stack.at(-1);
      if (node?.data !== undefined) {
        node.value = value;
        node.data.hProperties = { className: ["language-math", size === 2 ? "math-display" : "math-inline"] };
        node.data.hChildren = [{ type: "text", value }];
      }
      this.exit(token);
    },
    // The formula's lines and line endings collect in a buffer from the end of the opening fence on.
    omfpMathFlowFence(this: CompileContext): void {
      this.buffer();
    },
    omfpMathFlowValue(this: CompileContext, token: Token): void {
      this.config.enter.data?.call(this, token);
      this.config.exit.data?.call(this, token);
    },
    omfpMathFlow(this: CompileContext, token: Token): void {
      const value = this.resume().replace(EDGE_LINE_ENDINGS, "");
      const node = this.stack.at(-1);
      if (node?.data !== undefined) {
        node.value = value;
        const closed = node.data.closed === true ? { "data-fence-closed": true } : {};
        const properties = { className: ["language-math"], ...closed };
        node.data.hChildren = [{ type: "element", tagName: "code", properties, children: [{ type: "text", value }] }];
      }
      this.exit(token);
    },
  },
};

/** What a remark plugin's `this` offers: the processor's data, where the parser reads its extensions. */
interface Processor {
  data(key: string): unknown;
  data(key: string, value: unknown): unknown;
}

function extend(processor: Processor, key: string, extension: unknown): void {
  const extensions = processor.data(key);
  if (Array.isArray(extensions)) extensions.push(extension);
  else processor.data(key, [extension]);
}

/** The remark plugin: `$` math in text and `$$` blocks (UI-850, UI-851). */
export function remarkMath(this: Processor): void {
  extend(this, "micromarkExtensions", { text: { [DOLLAR]: mathText }, flow: { [DOLLAR]: mathFlow } });
  extend(this, "fromMarkdownExtensions", fromMarkdown);
}
