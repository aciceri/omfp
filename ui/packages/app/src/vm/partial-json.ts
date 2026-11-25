/**
 * The value of a JSON text the model is still writing (UI-054), in the manner of omp's `parseStreamingJson`
 * (`packages/utils/src/json-parse.ts`): a JSON prefix parsed as far as it goes. A string being written is the text so
 * far (an escape cut in the middle is left out), a number the digits so far; a key without its value yet, a literal
 * cut short and a lone minus sign are left out. Parsing stops at the first character that cannot continue the
 * JSON, keeping what came before it. One pass over the text, no exceptions.
 */

/** What parsing one value gave: the value (`undefined` when nothing usable yet) and whether its text ended. */
interface Parsed {
  readonly value: unknown;
  readonly complete: boolean;
}

const INCOMPLETE: Parsed = { value: undefined, complete: false };

const LITERALS: readonly (readonly [string, unknown])[] = [
  ["true", true],
  ["false", false],
  ["null", null],
];

/** Containers nested deeper than this end the parse (it recurses). */
const MAX_DEPTH = 1000;
/** The text of a JSON number, or of a prefix of one. */
const NUMBER = /-?(?:0|[1-9]\d*)?(?:\.\d*)?(?:[eE][+-]?\d*)?/y;
/** The longest whole JSON number at the start of a number's text. */
const WHOLE_NUMBER = /^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/;
/** The characters that end a run of plain string characters. */
const STRING_STOP = /["\\]/g;

const ESCAPES: Readonly<Record<string, string>> = {
  '"': '"',
  "\\": "\\",
  "/": "/",
  b: "\b",
  f: "\f",
  n: "\n",
  r: "\r",
  t: "\t",
};

/** The value of the JSON prefix `text`; `undefined` when it holds no value yet. */
export function parsePartialJson(text: string): unknown {
  return new PartialJson(text).value(0).value;
}

class PartialJson {
  private at = 0;

  constructor(private readonly text: string) {}

  value(depth: number): Parsed {
    this.skipSpace();
    const char = this.text[this.at];
    if (char === undefined) {
      return INCOMPLETE;
    }
    if (char === "{" || char === "[") {
      if (depth >= MAX_DEPTH) {
        return INCOMPLETE;
      }
      return char === "{" ? this.object(depth + 1) : this.array(depth + 1);
    }
    if (char === '"') {
      return this.string();
    }
    return char === "-" || (char >= "0" && char <= "9") ? this.number() : this.literal();
  }

  private skipSpace(): void {
    while (this.at < this.text.length) {
      const char = this.text[this.at];
      if (char !== " " && char !== "\t" && char !== "\n" && char !== "\r") {
        return;
      }
      this.at += 1;
    }
  }

  /** Whether the next character, after whitespace, is `char`; consumes it if so. */
  private take(char: string): boolean {
    this.skipSpace();
    if (this.text[this.at] !== char) {
      return false;
    }
    this.at += 1;
    return true;
  }

  private object(depth: number): Parsed {
    this.at += 1;
    const object: Record<string, unknown> = {};
    if (this.take("}")) {
      return { value: object, complete: true };
    }
    for (;;) {
      this.skipSpace();
      if (this.text[this.at] !== '"') {
        return { value: object, complete: false };
      }
      const key = this.string();
      if (!key.complete || typeof key.value !== "string" || !this.take(":")) {
        return { value: object, complete: false };
      }
      const entry = this.value(depth);
      if (entry.value !== undefined) {
        // Defined, not assigned: a `__proto__` key is an own key, as `JSON.parse` makes it.
        Object.defineProperty(object, key.value, {
          value: entry.value,
          enumerable: true,
          writable: true,
          configurable: true,
        });
      }
      if (!entry.complete || !this.take(",")) {
        return { value: object, complete: entry.complete && this.take("}") };
      }
    }
  }

  private array(depth: number): Parsed {
    this.at += 1;
    const array: unknown[] = [];
    if (this.take("]")) {
      return { value: array, complete: true };
    }
    for (;;) {
      const item = this.value(depth);
      if (item.value !== undefined) {
        array.push(item.value);
      }
      if (!item.complete || !this.take(",")) {
        return { value: array, complete: item.complete && this.take("]") };
      }
    }
  }

  private string(): Parsed {
    const { text } = this;
    this.at += 1;
    let value = "";
    for (;;) {
      STRING_STOP.lastIndex = this.at;
      const stop = STRING_STOP.exec(text);
      if (stop === null) {
        value += text.slice(this.at);
        this.at = text.length;
        return { value, complete: false };
      }
      value += text.slice(this.at, stop.index);
      this.at = stop.index + 1;
      if (stop[0] === '"') {
        return { value, complete: true };
      }
      const escaped = text[this.at];
      let unescaped: string | undefined;
      if (escaped === "u") {
        const hex = text.slice(this.at + 1, this.at + 5);
        unescaped = /^[0-9a-fA-F]{4}$/.test(hex) ? String.fromCharCode(Number.parseInt(hex, 16)) : undefined;
      } else if (escaped !== undefined) {
        unescaped = ESCAPES[escaped];
      }
      if (unescaped === undefined) {
        // An escape cut by the end of the text, or not JSON: the string ends before it.
        this.at = text.length;
        return { value, complete: false };
      }
      value += unescaped;
      this.at += escaped === "u" ? 5 : 1;
    }
  }

  private number(): Parsed {
    NUMBER.lastIndex = this.at;
    const match = NUMBER.exec(this.text)?.[0] ?? "";
    this.at += match.length;
    const whole = WHOLE_NUMBER.exec(match)?.[0];
    // A number running to the end of the text may go on; one followed by another character is whole.
    return { value: whole === undefined ? undefined : Number(whole), complete: this.at < this.text.length };
  }

  private literal(): Parsed {
    const rest = this.text.slice(this.at, this.at + 5);
    for (const [word, value] of LITERALS) {
      if (rest.startsWith(word)) {
        this.at += word.length;
        return { value, complete: true };
      }
      if (word.startsWith(rest)) {
        this.at = this.text.length;
        return INCOMPLETE;
      }
    }
    return INCOMPLETE;
  }
}
