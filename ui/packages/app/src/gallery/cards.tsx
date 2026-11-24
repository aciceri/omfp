/** The gallery's cards (UI-173): cards, tool cards, border boxes, interaction cards, diffs, code and Markdown. */
import {
  ApprovalCommand,
  BorderBox,
  BoxRule,
  Button,
  Card,
  Checkbox,
  Code,
  Count,
  Diff,
  DiffStat,
  ExitCode,
  InteractionActions,
  InteractionCard,
  Markdown,
  OptionText,
  Radio,
  RadioGroup,
  ResolvedLine,
  TextField,
  TodoList,
  ToolCard,
} from "@omfp/kit";
import { useState } from "react";
import { Line, Section } from "./parts.tsx";

export function Cards() {
  const [expanded, setExpanded] = useState(true);
  return (
    <Section title="Cards" note="UI-162">
      <Card title="Card" actions={<Count>3</Count>}>
        A transparent card with a rule border; a head and a body under a rule.
      </Card>
      <Card title="Collapsible" collapsible={{ expanded, onExpandedChange: setExpanded }}>
        Its head toggles the body.
      </Card>
      <Card title="Collapsed" collapsible={{ expanded: false, onExpandedChange: () => undefined }} />
      <Card title="Failed" tone="error">
        A toned card: the border takes the status color.
      </Card>
    </Section>
  );
}

const PATCH = `@@ -1,5 +1,6 @@ export function Button
 import { Kbd } from "./kbd.tsx";
-export type ButtonSize = "sm";
+export type ButtonSize = "sm" | "md";
+export type ButtonVariant = "primary" | "secondary";
 const DEFAULT = "secondary";
\u0020
 export function Button() {}
`;

const LS_OUTPUT = "app\nclient\ncore\nkit\nprotocol";

export function ToolCards() {
  const [open, setOpen] = useState(true);
  return (
    <Section title="Tool cards" note="UI-163 · UI-187 · expanded: head on the border">
      <ToolCard status="running" name="bash" arg="bun test packages/kit" meta="4.2s" expanded={false}>
        <span />
      </ToolCard>
      <ToolCard
        status="done"
        name="read"
        arg="ui/packages/kit/src/kit.css"
        meta="1,148 lines · focus"
        expanded={false}
        headClassName="is-focus"
      >
        <span />
      </ToolCard>
      <ToolCard
        status="done"
        name="bash"
        arg="ls -la ui/packages"
        meta={
          <>
            0.1s <ExitCode code={0} />
          </>
        }
        expanded={open}
        onExpandedChange={setOpen}
      >
        <Code block code="ls -la ui/packages" language="bash" />
        <BoxRule label="Output" />
        <Code block code={LS_OUTPUT} />
      </ToolCard>
      <ToolCard status="running" name="bash" arg="bun test packages/kit" meta="4.2s" expanded>
        <Code block code="bun test packages/kit" language="bash" />
        <BoxRule label="Output" />
        <Code block code={"bun test v1.4.0\n✓ kit styles [3.1ms]"} />
      </ToolCard>
      <ToolCard
        status="failed"
        name="bash"
        arg="cargo xtask spec-trace"
        meta={
          <>
            2.3s <ExitCode code={1} />
          </>
        }
        expanded
      >
        <Code block code="cargo xtask spec-trace" language="bash" />
        <BoxRule label="Output" />
        <Code block code="error: UI-174 is not cited by any test" />
      </ToolCard>
      <ToolCard status="done" name="write" arg="docs/notes.md" meta="3 lines · focus" expanded headClassName="is-focus">
        <Code block code={"# Notes\n\n- the title sits on the border"} language="markdown" />
      </ToolCard>
      <ToolCard status="done" name="todo" arg="4 items" expanded>
        <TodoList
          items={[
            { text: "Tokens in one namespace", status: "completed", statusLabel: "done" },
            { text: "Restyle the kit", status: "in_progress", statusLabel: "in progress" },
            { text: "Screenshot test", status: "pending", statusLabel: "pending" },
            { text: "Old palette", status: "abandoned", statusLabel: "abandoned" },
          ]}
        />
      </ToolCard>
      <ToolCard
        status="done"
        name="edit"
        arg="ui/packages/kit/src/button.tsx"
        meta={<DiffStat added={2} removed={1} />}
        expanded
      >
        <Diff patch={PATCH} language="ts" />
      </ToolCard>
      <ToolCard status="done" name="glob" arg="**/*.css" meta="no body" />
    </Section>
  );
}

/** Boxes with their title on the top border and labelled rules (UI-187). */
export function BorderBoxes() {
  return (
    <Section title="Border box" note="UI-187 · title on the border, labelled rule">
      <BorderBox title="Repositories" level={3}>
        A box whose title sits on its top border; the line never runs under the text.
      </BorderBox>
      <BorderBox title="Run a command?" end="0:42">
        Metadata may sit at the right end of the border.
        <BoxRule label="Output" />A labelled rule opens the next section.
      </BorderBox>
      <BorderBox title="A long title wraps where the border runs out of room; the line follows its first line">
        Body
      </BorderBox>
    </Section>
  );
}

export function Interactions() {
  return (
    <Section title="Approval · ask" note="UI-164">
      <InteractionCard title="Run a command?" meta="1/2" countdown="0:42">
        <ApprovalCommand command="rm -rf ui/packages/app/dist" />
        <InteractionActions>
          <Button variant="primary" size="sm" kbd="enter">
            Approve
          </Button>
          <Button variant="secondary" size="sm">
            Always
          </Button>
          <Button variant="danger" size="sm" kbd="esc">
            Deny
          </Button>
        </InteractionActions>
      </InteractionCard>
      <InteractionCard title="Run a command?" countdown="0:04" low>
        <ApprovalCommand command="git push --force" />
        <InteractionActions>
          <Button variant="primary" size="sm" isPending>
            Approve
          </Button>
          <Button variant="danger" size="sm" isDisabled>
            Deny
          </Button>
        </InteractionActions>
      </InteractionCard>
      <InteractionCard title="Which palette should the built-in theme use?" meta="2/3">
        <RadioGroup label="Palette" hideLabel defaultValue="synth">
          <Radio value="synth">
            <OptionText label="Synth" description="cyan into indigo, the default" />
          </Radio>
          <Radio value="lagoon">
            <OptionText label="Lagoon" description="teal-green into deep blue" />
          </Radio>
        </RadioGroup>
        <Checkbox defaultSelected>
          <OptionText label="Bundle the fonts" description="IBM Plex Serif and Commit Mono" />
        </Checkbox>
        <TextField label="Something else" hideLabel placeholder="Something else…" />
        <InteractionActions>
          <Button variant="primary" size="sm" kbd="enter">
            Answer
          </Button>
        </InteractionActions>
      </InteractionCard>
      <ResolvedLine outcome="approved" question="Run a command? rm -rf ui/packages/app/dist" answer="Approved" />
      <ResolvedLine outcome="denied" question="Run a command? git push --force" answer="Denied" />
      <ResolvedLine outcome="answered" question="Which palette should the built-in theme use?" answer="Synth" />
    </Section>
  );
}

export function Diffs() {
  return (
    <Section title="Diff" note="UI-165">
      <Diff patch={PATCH} language="ts" />
      <Diff patch={"@@ -0,0 +1,2 @@\n+added line one\n+added line two\n"} />
      <Diff patch={"@@ -1,2 +0,0 @@\n-removed line one\n-removed line two\n"} />
      <Line label="empty">
        <Diff patch="" />
      </Line>
      <Line label="binary">
        <Diff patch="" binary />
      </Line>
      <Line label="stats">
        <DiffStat added={2} removed={1} />
        <DiffStat added={1204} removed={0} />
      </Line>
    </Section>
  );
}

const MARKDOWN = `## Release plan

Here is the **plan**, starting with \`cargo xtask spec-trace\`: build the web bundle, then run the journeys and look at
every screenshot before calling it done.

- tokens in one namespace
- the kit restyled, then the app

\`\`\`rust
fn main() {
    println!("hello from the release plan");
}
\`\`\`
`;

/** TeX math (UI-850): inline and display, a color, prices that stay prose, and a formula KaTeX cannot parse. */
const MATH = String.raw`Euler's identity $e^{i\pi} + 1 = 0$ sits in the text, while the $5 and $10 plans stay prices.

$$
\sum_{n=1}^{\infty} \frac{1}{n^2} = \textcolor{teal}{\frac{\pi^2}{6}}
$$

A formula KaTeX cannot parse shows its source: $\frac{1}{$.
`;

export function CodeDemo() {
  return (
    <Section title="Code · prose" note="UI-166 · UI-144 · UI-850">
      <Code block language="ts" code={'// the built-in palette\nconst theme = "synth";\nexport default theme;'} />
      <p className="omfp-prose omfp-gallery__prose">
        Inline code like <Code code="--omfp-grad" /> sits in prose at 0.82em.
      </p>
      <Markdown text={MARKDOWN} />
      <Markdown text={MATH} />
    </Section>
  );
}
