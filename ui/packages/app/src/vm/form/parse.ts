/**
 * Ported from oh-my-pi v19.1.0 `packages/tui/src/html-form/parse.ts`.
 * MIT License, Copyright (c) 2025 Mario Zechner.
 *
 * Reduces an agent-written HTML `<form>` (a `form` request, UI-074) to what the card draws: titled sections (one per
 * top-level `<fieldset>`, plus untitled runs of loose content between them), context blocks (paragraphs, headings,
 * lists, preformatted text, tables, pictures) and fields. Radios and checkboxes join one {@link ChoiceField} per
 * `type` + `name` wherever they appear, so a group split across table cells or list items is still one question.
 * Layout-only markup (`<br>` between labels, wrapper `<div>`s, `style`, buttons) is dropped: the card owns layout and
 * Submit/Cancel. omp parses the same HTML to check the answer, so fields, names and option values must come out as
 * omp's parser finds them. The browser's `DOMParser` replaces omp's own DOM; nothing it parses is ever inserted into
 * the page.
 */

/** Inline emphasis a span keeps from `<code>`, `<strong>`/`<b>` and `<em>`/`<i>`. */
export type FormSpanStyle = "code" | "strong" | "em";

/** A run of inline text; `"\n"` inside `text` is a hard line break (`<br>`). */
export interface FormSpan {
  text: string;
  style?: FormSpanStyle;
}

/** One answer of a {@link ChoiceField}. */
export interface ChoiceOption {
  value: string;
  label: FormSpan[];
  checked: boolean;
  disabled: boolean;
}

/** A radio group (`multi: false`), checkbox group (`multi: true`) or `<select>` (`select: true`). */
export interface ChoiceField {
  kind: "choice";
  name: string;
  multi: boolean;
  /** From a `<select>`: a single select draws as a dropdown instead of option rows. */
  select: boolean;
  /**
   * The card adds a free-text "Other" answer: set for radio and checkbox groups of two or more options that don't
   * already offer one of their own.
   */
  other: boolean;
  /** The control's own label; empty for radio/checkbox groups, whose question is the surrounding text. */
  label: FormSpan[];
  options: ChoiceOption[];
  required: boolean;
}

/** A single-line `<input>` (`multiline: false`) or a `<textarea>`. */
export interface TextField {
  kind: "text";
  name: string;
  /** The `<input type>` (`text`, `number`, `url`, …); `textarea` for textareas. */
  type: string;
  multiline: boolean;
  label: FormSpan[];
  value: string;
  placeholder: string;
  required: boolean;
  min?: number;
  max?: number;
  pattern?: string;
  minLength?: number;
  maxLength?: number;
  /** Visible rows a textarea asked for (`rows`), 1 for inputs. */
  rows: number;
}

/** Any control the user answers. */
export type FormField = ChoiceField | TextField;

/** A renderable unit of a section, in document order. */
export type FormBlock =
  | { kind: "text"; spans: FormSpan[]; heading: boolean }
  | { kind: "list"; items: FormSpan[][]; ordered: boolean }
  | { kind: "pre"; text: string }
  | { kind: "table"; rows: FormSpan[][][]; headerRows: number }
  /** An `<img>`: `src` is an http(s) or `data:` URL, a `file:` URL or a local path. */
  | { kind: "image"; src: string; alt: string }
  /** An inline `<svg>`, serialized whole. */
  | { kind: "svg"; markup: string; alt: string }
  /**
   * A field. A radio/checkbox group whose later options come after other blocks of its section (text between
   * options) has `optionsEnd`: the index just past the last of those blocks.
   */
  | { kind: "field"; field: FormField; optionsEnd?: number };

/** A `<fieldset>` (titled by its `<legend>`) or loose content between fieldsets (untitled). */
export interface FormSection {
  title: FormSpan[];
  blocks: FormBlock[];
  /** A top-level `<fieldset>`, legend or not; `false` for loose content between fieldsets. */
  fieldset: boolean;
}

/** A parsed form: what to draw, the fields in document order, and `<input type="hidden">` pairs. */
export interface HtmlForm {
  sections: FormSection[];
  fields: FormField[];
  hidden: [name: string, value: string][];
}

type FieldBlock = Extract<FormBlock, { kind: "field" }>;

const SKIPPED: Readonly<Record<string, true>> = {
  button: true,
  head: true,
  iframe: true,
  link: true,
  meta: true,
  noscript: true,
  script: true,
  style: true,
  template: true,
  title: true,
};

const BLOCK: Readonly<Record<string, true>> = {
  address: true,
  article: true,
  aside: true,
  blockquote: true,
  dd: true,
  details: true,
  div: true,
  dl: true,
  dt: true,
  figure: true,
  figcaption: true,
  footer: true,
  header: true,
  li: true,
  main: true,
  nav: true,
  p: true,
  section: true,
};

const HEADING: Readonly<Record<string, true>> = {
  h1: true,
  h2: true,
  h3: true,
  h4: true,
  h5: true,
  h6: true,
  summary: true,
};

const STYLE_OF: Readonly<Record<string, FormSpanStyle>> = {
  b: "strong",
  code: "code",
  em: "em",
  i: "em",
  kbd: "code",
  samp: "code",
  strong: "strong",
  tt: "code",
};

const IGNORED_INPUTS: Readonly<Record<string, true>> = {
  button: true,
  file: true,
  image: true,
  reset: true,
  submit: true,
};

const CONTROL_SELECTOR = "input, select, textarea";

const ELEMENT_NODE = 1;
const TEXT_NODE = 3;
const CDATA_SECTION_NODE = 4;

function isElement(node: Node): node is Element {
  return node.nodeType === ELEMENT_NODE;
}

function isText(node: Node): node is CharacterData {
  return node.nodeType === TEXT_NODE || node.nodeType === CDATA_SECTION_NODE;
}

function childElements(element: Element): Element[] {
  return Array.from(element.children);
}

function queryAll(root: ParentNode, selector: string): Element[] {
  return Array.from(root.querySelectorAll(selector));
}

/** The label of a table control without its own: its row's text (and column header in a matrix), else the header. */
function rowLabel(row: FormSpan[], header: FormSpan[] | undefined, withHeader: boolean): FormSpan[] {
  if (row.length === 0) {
    return header ?? [];
  }
  const label = row.map((span) => ({ ...span }));
  if (withHeader && header !== undefined && header.length > 0) {
    appendSpan(label, " · ", undefined);
    for (const span of header) {
      appendSpan(label, span.text, span.style);
    }
  }
  return label;
}

/** Whether this page's `DOMParser` keeps the newline that opens a `<textarea>`'s content; probed once. */
let keepsTextareaNewline: boolean | undefined;

/** A textarea's initial value: its text without the one newline HTML drops after the start tag. */
function textareaValue(text: string): string {
  keepsTextareaNewline ??=
    new DOMParser().parseFromString("<textarea>\nx</textarea>", "text/html").querySelector("textarea")?.textContent ===
    "\nx";
  return keepsTextareaNewline ? text.replace(/^\r?\n/, "") : text;
}

/** Parses agent-written form HTML; content outside the first `<form>` is ignored when one exists. */
export function parseHtmlForm(html: string): HtmlForm {
  const document = new DOMParser().parseFromString(html, "text/html");
  return new FormBuilder(document.querySelector("form") ?? document).build();
}

/** Appends `text` to `spans`, merging into the previous span when the style matches. */
function appendSpan(spans: FormSpan[], text: string, style: FormSpanStyle | undefined): void {
  if (!text) {
    return;
  }
  const last = spans[spans.length - 1];
  if (last !== undefined && last.style === style) {
    last.text += text;
  } else {
    spans.push(style === undefined ? { text } : { text, style });
  }
}

/** Collapses whitespace like HTML layout does: runs become one space, spaces next to a hard break vanish, the ends are trimmed. Empty spans drop. */
function normalizeSpans(spans: readonly FormSpan[]): FormSpan[] {
  const out: FormSpan[] = [];
  for (const span of spans) {
    const text = span.text.replace(/[^\S\n]+/g, " ").replace(/ ?\n ?/g, "\n");
    appendSpan(out, text, span.style);
  }
  const first = out[0];
  if (first !== undefined) {
    first.text = first.text.replace(/^\s+/, "");
  }
  const last = out[out.length - 1];
  if (last !== undefined) {
    last.text = last.text.replace(/\s+$/, "");
  }
  return out.filter((span) => span.text !== "");
}

/** Plain text of spans, for labels that fall back to a single string. */
export function spansText(spans: readonly FormSpan[]): string {
  return spans.map((span) => span.text).join("");
}

/** An inline svg's accessible name: its `aria-label`, else its `<title>`. */
function svgTitle(svg: Element): string {
  const label = svg.getAttribute("aria-label");
  if (label) {
    return label.trim();
  }
  const title = childElements(svg).find((child) => child.localName === "title");
  return (title?.textContent ?? "").replace(/\s+/g, " ").trim();
}

function numberAttr(element: Element, name: string): number | undefined {
  const raw = element.getAttribute(name);
  if (raw === null || raw.trim() === "") {
    return undefined;
  }
  const value = Number(raw);
  return Number.isFinite(value) ? value : undefined;
}

/** The optional number attributes of a text field, present only when set (`exactOptionalPropertyTypes`). */
function numberAttrs<K extends string>(
  element: Element,
  attrs: Readonly<Record<K, string>>,
): Partial<Record<K, number>> {
  const out: Partial<Record<K, number>> = {};
  for (const key of Object.keys(attrs) as K[]) {
    const value = numberAttr(element, attrs[key]);
    if (value !== undefined) {
      out[key] = value;
    }
  }
  return out;
}

class FormBuilder {
  readonly #root: Document | Element;
  readonly #sections: FormSection[] = [];
  readonly #fields: FormField[] = [];
  readonly #hidden: [string, string][] = [];
  /** Radio/checkbox groups by `type\0name`, with the section and block of their first option. */
  readonly #groups = new Map<string, { field: ChoiceField; section: FormSection; block: FieldBlock }>();
  /** `<label for>` elements by the id of the control they name. */
  readonly #labelFor = new Map<string, Element>();
  #section: FormSection = { title: [], blocks: [], fieldset: false };
  #inline: FormSpan[] = [];
  #fieldsetDepth = 0;

  constructor(root: Document | Element) {
    this.#root = root;
    this.#sections.push(this.#section);
  }

  build(): HtmlForm {
    const ids = new Set<string>();
    for (const control of queryAll(this.#root, CONTROL_SELECTOR)) {
      if (control.id) {
        ids.add(control.id);
      }
    }
    for (const label of queryAll(this.#root, "label")) {
      const target = label.getAttribute("for");
      if (target && ids.has(target) && !this.#labelFor.has(target)) {
        this.#labelFor.set(target, label);
      }
    }
    this.#children(this.#root, undefined);
    this.#flush(false);
    for (const field of this.#fields) {
      if (field.kind !== "choice" || field.select || field.options.length < 2) {
        continue;
      }
      field.other = !field.options.some((option) => /^\s*other\b/i.test(spansText(option.label)));
    }
    return {
      sections: this.#sections.filter((section) => section.blocks.length > 0 || section.title.length > 0),
      fields: this.#fields,
      hidden: this.#hidden,
    };
  }

  #children(node: Node, style: FormSpanStyle | undefined): void {
    for (const child of Array.from(node.childNodes)) {
      this.#walk(child, style);
    }
  }

  #walk(node: Node, style: FormSpanStyle | undefined): void {
    if (isText(node)) {
      appendSpan(this.#inline, node.data, style);
      return;
    }
    if (!isElement(node)) {
      return;
    }
    const tag = node.localName;
    if (SKIPPED[tag]) {
      return;
    }
    switch (tag) {
      case "br":
        appendSpan(this.#inline, "\n", undefined);
        return;
      case "hr":
        this.#flush(false);
        return;
      case "input":
      case "select":
      case "textarea":
        this.#control(node, undefined);
        return;
      case "label":
        this.#label(node, style);
        return;
      case "fieldset":
        this.#fieldset(node);
        return;
      case "legend":
        this.#flush(false);
        this.#children(node, style);
        this.#flush(true);
        return;
      case "ul":
      case "ol":
        this.#list(node, tag === "ol");
        return;
      case "pre":
        this.#flush(false);
        this.#section.blocks.push({ kind: "pre", text: (node.textContent ?? "").replace(/^\n/, "").trimEnd() });
        return;
      case "table":
        this.#table(node);
        return;
      case "img": {
        const src = node.getAttribute("src")?.trim();
        if (!src) {
          return;
        }
        this.#flush(false);
        this.#section.blocks.push({ kind: "image", src, alt: (node.getAttribute("alt") ?? "").trim() });
        return;
      }
      case "svg":
        this.#flush(false);
        this.#section.blocks.push({ kind: "svg", markup: node.outerHTML, alt: svgTitle(node) });
        return;
    }
    if (HEADING[tag]) {
      this.#flush(false);
      this.#children(node, style);
      this.#flush(true);
      return;
    }
    if (BLOCK[tag]) {
      this.#flush(false);
      this.#children(node, style);
      this.#flush(false);
      return;
    }
    this.#children(node, STYLE_OF[tag] ?? style);
  }

  /** Emits buffered inline content as a text block. */
  #flush(heading: boolean): void {
    const spans = normalizeSpans(this.#inline);
    this.#inline = [];
    if (spans.length > 0) {
      this.#section.blocks.push({ kind: "text", spans, heading });
    }
  }

  /** Inline spans of `element`, skipping controls (their label is the text around them). */
  #spansOf(element: Element): FormSpan[] {
    const spans: FormSpan[] = [];
    const visit = (node: Node, style: FormSpanStyle | undefined): void => {
      if (isText(node)) {
        appendSpan(spans, node.data, style);
        return;
      }
      if (!isElement(node)) {
        return;
      }
      const tag = node.localName;
      if (SKIPPED[tag] || tag === "input" || tag === "select" || tag === "textarea" || tag === "svg") {
        return;
      }
      if (tag === "br") {
        appendSpan(spans, "\n", undefined);
        return;
      }
      // Labels stay text: a picture inside one reads as its alt text.
      if (tag === "img") {
        appendSpan(spans, node.getAttribute("alt") ?? "", style);
        return;
      }
      if (BLOCK[tag] || HEADING[tag]) {
        appendSpan(spans, " ", undefined);
      }
      for (const child of Array.from(node.childNodes)) {
        visit(child, STYLE_OF[tag] ?? style);
      }
    };
    for (const child of Array.from(element.childNodes)) {
      visit(child, undefined);
    }
    return normalizeSpans(spans);
  }

  #label(label: Element, style: FormSpanStyle | undefined): void {
    const controls = queryAll(label, CONTROL_SELECTOR);
    // A detached `<label for>` is drawn by the control it names; one wrapping its control is an ordinary label.
    const target = label.getAttribute("for");
    if (controls.length === 0 && target && this.#labelFor.get(target) === label) {
      return;
    }
    if (controls.length === 0) {
      this.#children(label, style);
      return;
    }
    this.#flush(false);
    const spans = this.#spansOf(label);
    for (const control of controls) {
      this.#control(control, spans);
    }
  }

  #fieldset(fieldset: Element): void {
    this.#flush(false);
    const legend = childElements(fieldset).find((child) => child.localName === "legend");
    if (this.#fieldsetDepth > 0) {
      // Nested fieldsets stay in the outer section; their legend becomes a heading.
      if (legend !== undefined) {
        this.#walk(legend, undefined);
      }
    } else {
      this.#startSection(legend === undefined ? [] : this.#spansOf(legend), true);
    }
    this.#fieldsetDepth++;
    for (const child of Array.from(fieldset.childNodes)) {
      if (child !== legend) {
        this.#walk(child, undefined);
      }
    }
    this.#flush(false);
    this.#fieldsetDepth--;
    if (this.#fieldsetDepth === 0) {
      this.#startSection([], false);
    }
  }

  #startSection(title: FormSpan[], fieldset: boolean): void {
    if (this.#section.blocks.length === 0 && this.#section.title.length === 0) {
      this.#section.title = title;
      this.#section.fieldset = fieldset;
      return;
    }
    this.#section = { title, blocks: [], fieldset };
    this.#sections.push(this.#section);
  }

  #list(list: Element, ordered: boolean): void {
    this.#flush(false);
    if (list.querySelector(CONTROL_SELECTOR) !== null) {
      // A list of controls is layout, not content: walk it as flow.
      this.#children(list, undefined);
      this.#flush(false);
      return;
    }
    const items = childElements(list)
      .filter((child) => child.localName === "li")
      .map((item) => this.#spansOf(item))
      .filter((spans) => spans.length > 0);
    if (items.length > 0) {
      this.#section.blocks.push({ kind: "list", items, ordered });
    }
  }

  #table(table: Element): void {
    this.#flush(false);
    const rows = queryAll(table, "tr").filter((row) => row.closest("table") === table);
    const cellsOf = (row: Element) =>
      childElements(row).filter((cell) => cell.localName === "td" || cell.localName === "th");
    if (table.querySelector(CONTROL_SELECTOR) === null) {
      const cells = rows.map((row) => cellsOf(row).map((cell) => this.#spansOf(cell)));
      let headerRows = 0;
      for (const row of rows) {
        if (row.closest("thead") === null && !cellsOf(row).every((cell) => cell.localName === "th")) {
          break;
        }
        headerRows++;
      }
      if (cells.some((row) => row.length > 0)) {
        this.#section.blocks.push({ kind: "table", rows: cells, headerRows });
      }
      return;
    }
    // A table of controls (a decision matrix): a control without a label of its own (its `<label>`, else its cell's
    // text) is named after its row's text, with its column header when the row has controls in several columns, so
    // each option keeps the row it answers when a group's options from many rows are drawn together; without row
    // text it takes its column header. The row's text stays a line of its own when the row has no control (a price
    // or feature row the choice depends on) or some control keeps its own label. (omp 19.1.0 names such a control
    // after its header alone, which loses the row in a card.)
    let header: FormSpan[][] = [];
    for (const row of rows) {
      const cells = cellsOf(row);
      if (cells.every((cell) => cell.localName === "th") && row.querySelector(CONTROL_SELECTOR) === null) {
        header = cells.map((cell) => this.#spansOf(cell));
        continue;
      }
      const text: FormSpan[] = [];
      for (const cell of cells) {
        if (cell.querySelector(CONTROL_SELECTOR) !== null) {
          continue;
        }
        const spans = this.#spansOf(cell);
        if (spans.length === 0) {
          continue;
        }
        if (text.length > 0) {
          appendSpan(text, " · ", undefined);
        }
        for (const span of spans) {
          appendSpan(text, span.text, span.style);
        }
      }
      const controls = cells.flatMap((cell, column) =>
        queryAll(cell, CONTROL_SELECTOR).map((control) => {
          const own = control.closest("label");
          return {
            control,
            column,
            spans: own !== null && cell.contains(own) ? this.#spansOf(own) : this.#spansOf(cell),
          };
        }),
      );
      const columns = new Set(controls.map(({ column }) => column)).size;
      const labelled = controls.length === 0 || controls.some(({ spans }) => spans.length > 0);
      if (text.length > 0 && labelled) {
        this.#section.blocks.push({ kind: "text", spans: text, heading: false });
      }
      for (const { control, column, spans } of controls) {
        if (spans.length > 0) {
          this.#control(control, spans);
          continue;
        }
        // A label of its own outside the cell (`<label for>`, `aria-label`, `title`) says what the control does; the
        // row before it says which row it answers.
        const label = [...rowLabel(text, header[column], columns > 1)];
        const own = this.#detachedLabel(control);
        if (label.length > 0 && own.length > 0) {
          appendSpan(label, " · ", undefined);
        }
        for (const span of own) {
          appendSpan(label, span.text, span.style);
        }
        this.#control(control, label);
      }
    }
  }

  /** Label of a control outside any wrapping `<label>`: `<label for>`, then `aria-label`, `title`. */
  #detachedLabel(control: Element): FormSpan[] {
    const label = control.id ? this.#labelFor.get(control.id) : undefined;
    if (label !== undefined) {
      return this.#spansOf(label);
    }
    const text = control.getAttribute("aria-label") ?? control.getAttribute("title");
    return text ? normalizeSpans([{ text }]) : [];
  }

  #control(control: Element, wrapping: FormSpan[] | undefined): void {
    this.#flush(false);
    const label = wrapping !== undefined && wrapping.length > 0 ? wrapping : this.#detachedLabel(control);
    const name = control.getAttribute("name") ?? "";
    const required = control.hasAttribute("required");
    const tag = control.localName;
    if (tag === "select") {
      const multi = control.hasAttribute("multiple");
      const options: ChoiceOption[] = queryAll(control, "option").map((option) => {
        const text = (option.textContent ?? "").replace(/\s+/g, " ").trim();
        return {
          value: option.getAttribute("value") ?? text,
          label: [{ text }],
          checked: option.hasAttribute("selected"),
          disabled: option.hasAttribute("disabled"),
        };
      });
      // A single select always has a selection: the first enabled option unless one is marked.
      if (!multi && !options.some((option) => option.checked)) {
        const first = options.find((option) => !option.disabled);
        if (first !== undefined) {
          first.checked = true;
        }
      }
      this.#addField({ kind: "choice", name, multi, select: true, other: false, label, options, required });
      return;
    }
    if (tag === "textarea") {
      this.#addField({
        kind: "text",
        name,
        type: "textarea",
        multiline: true,
        label: label.length > 0 ? label : this.#nameLabel(name),
        // HTML drops one newline right after `<textarea>`: browsers' parsers do it themselves, happy-dom's (and
        // omp's) leave it to the reader. Dropped here only when this parser kept it, so it goes exactly once.
        value: textareaValue(control.textContent ?? ""),
        placeholder: control.getAttribute("placeholder") ?? "",
        required,
        ...numberAttrs(control, { minLength: "minlength", maxLength: "maxlength" }),
        rows: numberAttr(control, "rows") ?? 4,
      });
      return;
    }
    const type = (control.getAttribute("type") ?? "text").toLowerCase();
    if (IGNORED_INPUTS[type]) {
      return;
    }
    const value = control.getAttribute("value");
    if (type === "hidden") {
      if (name) {
        this.#hidden.push([name, value ?? ""]);
      }
      return;
    }
    if (type === "radio" || type === "checkbox") {
      const option: ChoiceOption = {
        value: value ?? "on",
        label: label.length > 0 ? label : [{ text: value ?? name }],
        checked: control.hasAttribute("checked"),
        disabled: control.hasAttribute("disabled"),
      };
      const key = `${type}\0${name}`;
      const group = this.#groups.get(key);
      if (group !== undefined) {
        group.field.options.push(option);
        group.field.required ||= required;
        if (group.section === this.#section && this.#section.blocks.at(-1) !== group.block) {
          group.block.optionsEnd = this.#section.blocks.length;
        }
        return;
      }
      const field: ChoiceField = {
        kind: "choice",
        name,
        multi: type === "checkbox",
        select: false,
        other: false,
        label: [],
        options: [option],
        required,
      };
      this.#groups.set(key, { field, section: this.#section, block: this.#addField(field) });
      return;
    }
    const pattern = control.getAttribute("pattern");
    this.#addField({
      kind: "text",
      name,
      type,
      multiline: false,
      label: label.length > 0 ? label : this.#nameLabel(name),
      value: value ?? "",
      placeholder: control.getAttribute("placeholder") ?? "",
      required,
      ...numberAttrs(control, { min: "min", max: "max", minLength: "minlength", maxLength: "maxlength" }),
      ...(pattern === null ? {} : { pattern }),
      rows: 1,
    });
  }

  /** Last-resort label from the control name: `sentry_dsn` → `sentry dsn`. */
  #nameLabel(name: string): FormSpan[] {
    return name ? [{ text: name.replace(/[_-]+/g, " ") }] : [];
  }

  #addField(field: FormField): FieldBlock {
    const block: FieldBlock = { kind: "field", field };
    this.#fields.push(field);
    this.#section.blocks.push(block);
    return block;
  }
}
