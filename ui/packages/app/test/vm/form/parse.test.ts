import { describe, expect, test } from "bun:test";
import { type ChoiceField, parseHtmlForm, spansText, type TextField } from "../../../src/vm/form/parse.ts";

function choice(html: string, at = 0): ChoiceField {
  const field = parseHtmlForm(html).fields[at];
  if (field?.kind !== "choice") {
    throw new Error("not a choice field");
  }
  return field;
}

function text(html: string, at = 0): TextField {
  const field = parseHtmlForm(html).fields[at];
  if (field?.kind !== "text") {
    throw new Error("not a text field");
  }
  return field;
}

describe("form parse", () => {
  // spec: UI-074
  test("fieldsets become titled sections, loose content untitled ones, context blocks keep their order", () => {
    const form = parseHtmlForm(`
      <p>Before the form</p>
      <form>
        <h2>Setup</h2>
        <p>Pick <code>one</code> <strong>now</strong><br>or <em>later</em></p>
        <fieldset><legend>Database</legend>
          <ul><li>fast</li><li>small</li></ul>
          <ol><li>first</li></ol>
          <pre>
  line 1
  line 2  </pre>
          <table><thead><tr><th>Name</th><th>Size</th></tr></thead><tr><td>a</td><td>1</td></tr></table>
          <img src="https://example.com/a.png" alt="A chart">
          <svg aria-label="Logo"><circle r="1"/></svg>
        </fieldset>
        <p>After</p>
      </form>`);
    expect(form.sections.map((section) => [spansText(section.title), section.fieldset])).toEqual([
      ["", false],
      ["Database", true],
      ["", false],
    ]);
    const [intro, database, after] = form.sections;
    expect(intro?.blocks).toEqual([
      { kind: "text", spans: [{ text: "Setup" }], heading: true },
      {
        kind: "text",
        spans: [
          { text: "Pick " },
          { text: "one", style: "code" },
          { text: " " },
          { text: "now", style: "strong" },
          { text: "\nor " },
          { text: "later", style: "em" },
        ],
        heading: false,
      },
    ]);
    const blocks = database?.blocks ?? [];
    expect(blocks.map((block) => block.kind)).toEqual(["list", "list", "pre", "table", "image", "svg"]);
    expect(blocks[0]).toEqual({ kind: "list", items: [[{ text: "fast" }], [{ text: "small" }]], ordered: false });
    expect(blocks[1]).toEqual({ kind: "list", items: [[{ text: "first" }]], ordered: true });
    expect(blocks[2]).toEqual({ kind: "pre", text: "  line 1\n  line 2" });
    expect(blocks[3]).toEqual({
      kind: "table",
      rows: [
        [[{ text: "Name" }], [{ text: "Size" }]],
        [[{ text: "a" }], [{ text: "1" }]],
      ],
      headerRows: 1,
    });
    expect(blocks[4]).toEqual({ kind: "image", src: "https://example.com/a.png", alt: "A chart" });
    expect(blocks[5]?.kind === "svg" ? [blocks[5].alt, blocks[5].markup.startsWith("<svg")] : []).toEqual([
      "Logo",
      true,
    ]);
    expect(after?.blocks).toEqual([{ kind: "text", spans: [{ text: "After" }], heading: false }]);
  });

  // spec: UI-074
  test("radios and checkboxes join one group per type and name across markup; selects and text fields keep their rules", () => {
    const form = parseHtmlForm(`<form>
      <table>
        <tr><th>Tier</th><th>Pick</th></tr>
        <tr><td>Free</td><td><input type="radio" name="tier" value="free" checked></td></tr>
        <tr><td>Pro</td><td><input type="radio" name="tier" value="pro" disabled></td></tr>
      </table>
      <ul>
        <li><label><input type="checkbox" name="extras" value="a" required> A</label></li>
        <li><label><input type="checkbox" name="extras" value="b" checked> B</label></li>
      </ul>
      <label for="db">Database</label>
      <select id="db" name="db"><option disabled>none</option><option value="pg">Postgres</option><option>SQLite</option></select>
      <select name="langs" multiple aria-label="Languages"><option value="rs">Rust</option><option value="ts" selected>TS</option></select>
      <label>Port <input type="number" name="port" min="1" max="65535" value="8080" required></label>
      <textarea name="notes_text" rows="3" minlength="2" maxlength="9">
hello</textarea>
      <input type="email" name="mail" title="Mail">
      <input type="submit" value="Send"><button>Go</button>
    </form>`);
    expect(form.fields.map((field) => [field.kind, field.name])).toEqual([
      ["choice", "tier"],
      ["choice", "extras"],
      ["choice", "db"],
      ["choice", "langs"],
      ["text", "port"],
      ["text", "notes_text"],
      ["text", "mail"],
    ]);
    const [tier, extras, db, langs, port, notes, mail] = form.fields;
    expect(tier).toMatchObject({ multi: false, select: false, other: true, required: false });
    expect(
      tier?.kind === "choice" ? tier.options.map((option) => [option.value, option.checked, option.disabled]) : [],
    ).toEqual([
      ["free", true, false],
      ["pro", false, true],
    ]);
    // An unlabeled control in a table is named after its row, so each option keeps the row it answers when the
    // group's options are drawn together; the row's text needs no line of its own then.
    expect(tier?.kind === "choice" ? tier.options.map((option) => spansText(option.label)) : []).toEqual([
      "Free",
      "Pro",
    ]);
    expect(
      form.sections[0]?.blocks.flatMap((block) => (block.kind === "text" ? [spansText(block.spans)] : [])),
    ).toEqual([]);
    expect(extras).toMatchObject({ multi: true, required: true, other: true });
    // A single select without a marked option selects its first enabled one, and offers no "Other".
    expect(db).toMatchObject({ select: true, multi: false, other: false, label: [{ text: "Database" }] });
    expect(db?.kind === "choice" ? db.options.map((option) => [option.value, option.checked]) : []).toEqual([
      ["none", false],
      ["pg", true],
      ["SQLite", false],
    ]);
    expect(langs).toMatchObject({ select: true, multi: true, other: false, label: [{ text: "Languages" }] });
    expect(port).toMatchObject({ type: "number", value: "8080", min: 1, max: 65535, required: true, rows: 1 });
    expect(notes).toMatchObject({
      type: "textarea",
      multiline: true,
      value: "hello",
      minLength: 2,
      maxLength: 9,
      rows: 3,
      label: [{ text: "notes text" }],
    });
    expect(mail).toMatchObject({ type: "email", label: [{ text: "Mail" }], required: false });
  });

  // spec: UI-074
  test("a table control with a `<label for>` outside its cell keeps that label, after its row", () => {
    const tls = choice(`<form>
      <label for="tls">Enable TLS</label>
      <table><tr><td>Production</td><td><input type="checkbox" id="tls" name="tls" value="on"></td></tr></table>
    </form>`);
    expect(spansText(tls.options[0]?.label ?? [])).toBe("Production · Enable TLS");
  });

  // spec: UI-074
  test("a `<label for>` wrapping its own control labels it like any wrapping label", () => {
    const name = text(`<form><label for="name">Name <input id="name" name="name" required></label></form>`);
    expect([name.name, spansText(name.label), name.required]).toEqual(["name", "Name", true]);
  });

  // spec: UI-074
  test("hidden inputs are kept as pairs; content outside the form is ignored", () => {
    const form = parseHtmlForm(
      `<p>outside</p><form><input type="hidden" name="token" value="t1"><input type="hidden" name="empty"><input type="hidden" value="x"></form>`,
    );
    expect(form.hidden).toEqual([
      ["token", "t1"],
      ["empty", ""],
    ]);
    expect(form.fields).toEqual([]);
    expect(form.sections).toEqual([]);
  });

  // spec: UI-074
  test("groups of two or more radio or checkbox options offer Other unless one is labelled Other", () => {
    const radios = (labels: string[]) =>
      labels.map((label) => `<label><input type="radio" name="r" value="${label}"> ${label}</label>`).join("");
    expect(choice(`<form>${radios(["a", "b"])}</form>`).other).toBe(true);
    expect(choice(`<form>${radios(["a", "Other (say)"])}</form>`).other).toBe(false);
    expect(choice(`<form>${radios(["a", "others"])}</form>`).other).toBe(true);
    expect(choice(`<form>${radios(["a"])}</form>`).other).toBe(false);
    expect(choice(`<form><label><input type="checkbox" name="agree"> I agree</label></form>`)).toMatchObject({
      multi: true,
      other: false,
      options: [{ value: "on", label: [{ text: "I agree" }] }],
    });
  });

  // spec: UI-074
  test("a text field's pattern and limits come from its attributes, invalid numbers are dropped", () => {
    expect(text(`<input name="code" pattern="[A-Z]{3}" minlength="3" maxlength="x" min="">`)).toMatchObject({
      pattern: "[A-Z]{3}",
      minLength: 3,
    });
    const field = text(`<input name="code" maxlength="x" min="">`);
    expect([field.maxLength, field.min, field.pattern]).toEqual([undefined, undefined, undefined]);
  });

  // spec: UI-074
  test("a matrix row names its unlabeled controls after the row and their column", () => {
    const form = parseHtmlForm(`<form><table>
      <tr><th>Service</th><th>Dev</th><th>Prod</th></tr>
      <tr><td>API</td><td><input type="checkbox" name="api" value="dev"></td><td><input type="checkbox" name="api" value="prod"></td></tr>
      <tr><td>Web</td><td><label><input type="checkbox" name="web" value="dev"> staging</label></td><td><input type="checkbox" name="web" value="prod"></td></tr>
    </table></form>`);
    expect(
      form.fields.map((field) =>
        field.kind === "choice" ? field.options.map((option) => spansText(option.label)) : [],
      ),
    ).toEqual([
      ["API · Dev", "API · Prod"],
      ["staging", "Web · Prod"],
    ]);
    // A row where a control keeps its own label still shows the row's text as context.
    expect(
      form.sections[0]?.blocks.flatMap((block) => (block.kind === "text" ? [spansText(block.spans)] : [])),
    ).toEqual(["Web"]);
  });

  // spec: UI-074
  test("rows without controls in a table of controls stay as context before the choice", () => {
    const form = parseHtmlForm(`<form><table>
      <tr><th></th><th>Free</th><th>Pro</th></tr>
      <tr><td>Price</td><td>0 €</td><td>9 €</td></tr>
      <tr><td>Seats</td><td>1</td><td>10</td></tr>
      <tr><td>Choose</td><td><input type="radio" name="plan" value="free"></td><td><input type="radio" name="plan" value="pro"></td></tr>
    </table></form>`);
    expect(
      form.sections[0]?.blocks.flatMap((block) => (block.kind === "text" ? [spansText(block.spans)] : [])),
    ).toEqual(["Price · 0 € · 9 €", "Seats · 1 · 10"]);
    expect(form.fields.length).toBe(1);
    const plan = form.fields[0];
    expect(plan?.kind === "choice" ? plan.options.map((option) => spansText(option.label)) : []).toEqual([
      "Choose · Free",
      "Choose · Pro",
    ]);
  });
});
