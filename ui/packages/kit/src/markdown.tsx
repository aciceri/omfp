import { type ComponentProps, type ComponentType, type HTMLAttributes, type ReactNode, useMemo } from "react";
import ReactMarkdown, {
  type Components,
  defaultUrlTransform,
  type ExtraProps,
  type UrlTransform,
} from "react-markdown";
import remarkGfm from "remark-gfm";
import { cx } from "./class.ts";
import { Highlighted } from "./highlight.tsx";
import { remarkMath } from "./math/syntax.ts";
import { MathFormula } from "./math.tsx";
import { MermaidDiagram } from "./mermaid.tsx";
import {
  absoluteSource,
  linkReference,
  type OpenPath,
  PathLink,
  pathReference,
  relativeSource,
  resolvePath,
} from "./paths.tsx";
import { threadLinkId } from "./thread-links.ts";

const REMARK_PLUGINS = [remarkGfm, remarkClosedFences, remarkMath];

const FENCE_LANGUAGE = /(?:^|\s)language-(\S+)/;
const MATH_CLASS = /(?:^|\s)math-(inline|display)(?:\s|$)/;

/** The fences drawn only once their closing fence is parsed: diagrams (UI-981) and math (UI-850). */
const DRAWN_FENCES: Readonly<Record<string, true>> = { mermaid: true, math: true };

/** The fence language of a `<pre>`'s `<code>` child (its `language-*` class), if any. */
function fenceLanguage(node: { children?: readonly unknown[] } | undefined): string | undefined {
  const code = node?.children?.[0];
  if (typeof code !== "object" || code === null || !("properties" in code)) {
    return undefined;
  }
  const names = (code.properties as { className?: unknown }).className;
  const name = Array.isArray(names)
    ? names.find((value) => typeof value === "string" && value.startsWith("language-"))
    : undefined;
  return typeof name === "string" ? name.slice("language-".length) : undefined;
}

interface FenceContext {
  data: { flowCodeInside?: boolean };
  stack: { type: string; lang?: string; data?: { hProperties?: Record<string, unknown> } }[];
}

/** Marks the code of a closed `mermaid` or `math` fence `data-fence-closed`, so a streaming open one stays code. */
function remarkClosedFences(this: {
  data(key: string): unknown;
  data(key: string, value: unknown): unknown;
}) {
  const extension = {
    enter: {
      codeFencedFence(this: FenceContext) {
        // This hook runs for both fences; the default exit handler starts buffering after the opening one.
        if (!this.data.flowCodeInside) return;
        const code = this.stack.findLast((node) => node.type === "code");
        if (code === undefined || DRAWN_FENCES[code.lang?.toLowerCase() ?? ""] !== true) return;
        code.data ??= {};
        code.data.hProperties ??= {};
        code.data.hProperties["data-fence-closed"] = true;
      },
    },
  };
  const extensions = this.data("fromMarkdownExtensions");
  if (Array.isArray(extensions)) extensions.push(extension);
  else this.data("fromMarkdownExtensions", [extension]);
}

/** A fenced code block, highlighted (UI-090); a closed `mermaid` block is drawn instead (UI-981), a closed `math` fence
 * or `$$` block typeset (UI-850). */
function FencedBlock({ node, className, ...props }: ComponentProps<"pre"> & ExtraProps) {
  const fence = fenceLanguage(node);
  const language = fence?.toLowerCase() ?? "";
  const code = node?.children[0];
  if (DRAWN_FENCES[language] === true && code?.type === "element" && code.properties["data-fence-closed"] === true) {
    const text = code.children.map((child) => (child.type === "text" ? child.value : "")).join("");
    const source = text.replace(/\n$/, "");
    if (language === "math") {
      return <MathFormula source={source} display />;
    }
    return <MermaidDiagram source={source} />;
  }
  return <pre {...props} className={cx("omfp-code omfp-code--block", className)} data-lang={fence} />;
}

/** Links open in a new tab without leaking the referrer, a thread link (UI-045) is its text; fenced code with a
 * language is highlighted (UI-090), a closed `mermaid` fence drawn (UI-981); math is typeset (UI-850); a table sits in a
 * region that scrolls sideways when it is wider than the text (UI-369), focusable so the keyboard can scroll it;
 * everything else keeps react-markdown's elements. */
const COMPONENTS = {
  a: ({ node: _node, ...props }) =>
    threadLinkId(props.href) === undefined ? (
      <a {...props} target="_blank" rel="noreferrer" />
    ) : (
      <span data-testid="thread-link-text">{props.children}</span>
    ),
  pre: FencedBlock,
  table: ({ node: _node, ...props }) => (
    // biome-ignore lint/a11y/noNoninteractiveTabindex: a scrollable region must be focusable to scroll by keyboard.
    <section className="omfp-markdown__table" aria-label="Table" tabIndex={0}>
      <table {...props} />
    </section>
  ),
  code: ({ node: _node, className, children, ...props }) => {
    const math = MATH_CLASS.exec(className ?? "")?.[1];
    if (math !== undefined && typeof children === "string") {
      return <MathFormula source={children} display={math === "display"} />;
    }
    const language = FENCE_LANGUAGE.exec(className ?? "")?.[1];
    return (
      <code {...props} className={className}>
        {language !== undefined && typeof children === "string" ? (
          <Highlighted code={children} language={language} />
        ) : (
          children
        )}
      </code>
    );
  },
} satisfies Components;

/** The kit's element overrides: {@link COMPONENTS} and the layers over it, each `a` a component of its own (never a
 * tag name), so a layer can render the one it wraps. */
type MarkdownComponents = typeof COMPONENTS;

/** {@link COMPONENTS} where file paths open with `onOpen` (UI-116): inline code that reads as a path, and links to a
 * relative path (resolved against `linkBase` when given, UI-561). A fenced block's text ends with a line break, so it
 * never counts as inline code. */
function withPaths(onOpen: OpenPath, linkBase: string | undefined): MarkdownComponents {
  const { a: Link, code: CodeElement } = COMPONENTS;
  return {
    ...COMPONENTS,
    a: (props) => {
      const target = linkReference(props.href);
      const reference =
        target === undefined || linkBase === undefined
          ? target
          : { path: resolvePath(linkBase, target.path), line: target.line };
      if (reference === undefined) {
        return <Link {...props} />;
      }
      return (
        <PathLink reference={reference} onOpen={onOpen}>
          {props.children}
        </PathLink>
      );
    },
    code: (props) => {
      const { children, className } = props;
      const reference =
        typeof children === "string" && !children.includes("\n") && className === undefined
          ? pathReference(children)
          : undefined;
      if (reference === undefined) {
        return <CodeElement {...props} />;
      }
      return (
        <PathLink reference={reference} onOpen={onOpen}>
          <code>{children}</code>
        </PathLink>
      );
    },
  };
}

/** A link to a thread in Markdown (UI-045, UI-046): `omfp://thread/<id>`. */
export interface MarkdownThreadLinkProps {
  /** The id the link names. */
  readonly threadId: string;
  /** The link's text is the bare id or the link itself (`<omfp://thread/<id>>`), so it should show the thread's own
   * title instead. */
  readonly bare: boolean;
  /** The link's text as written. */
  readonly children: ReactNode;
}

/** `components` where links to a thread render with `ThreadLink` (UI-045); other links stay as they were. */
function withThreads(
  components: MarkdownComponents,
  ThreadLink: ComponentType<MarkdownThreadLinkProps>,
): MarkdownComponents {
  const { a: Link } = components;
  return {
    ...components,
    a: (props) => {
      const threadId = threadLinkId(props.href);
      if (threadId === undefined) {
        return <Link {...props} />;
      }
      const text = plainText(props.children);
      return (
        <ThreadLink threadId={threadId} bare={text === threadId || text === props.href}>
          {props.children}
        </ThreadLink>
      );
    },
  };
}

/** The text of `children` when it is only text, else `undefined`. */
function plainText(children: ReactNode): string | undefined {
  if (typeof children === "string") {
    return children.trim();
  }
  return Array.isArray(children) && children.every((child) => typeof child === "string")
    ? children.join("").trim()
    : undefined;
}

/** react-markdown's URL filter (no `javascript:` links, UI-040), keeping the links to a thread (UI-045). */
const urlTransform: UrlTransform = (url, key) =>
  key === "href" && threadLinkId(url) !== undefined ? url : defaultUrlTransform(url);

/** A Markdown image whose source is a file path (UI-562, UI-572). */
export interface MarkdownImageProps {
  /** The file's path: a relative source resolved against `linkBase`, an absolute one as written. */
  readonly path: string;
  readonly alt: string;
  readonly title: string | undefined;
}

/** `components` where relative images, and with `absolute` absolute ones, render with `Image` (UI-562, UI-572); other
 * images stay react-markdown's `<img>`. */
function withImage(
  components: Components,
  Image: ComponentType<MarkdownImageProps>,
  linkBase: string,
  absolute: boolean,
): Components {
  return {
    ...components,
    img: ({ node: _node, ...props }) => {
      const relative = relativeSource(props.src);
      const path =
        relative !== undefined ? resolvePath(linkBase, relative) : absolute ? absoluteSource(props.src) : undefined;
      if (path === undefined) {
        return <img {...props} alt={props.alt ?? ""} />;
      }
      return <Image path={path} alt={props.alt ?? ""} title={props.title} />;
    },
  };
}

export interface MarkdownProps extends Omit<HTMLAttributes<HTMLDivElement>, "children"> {
  /** Markdown source (CommonMark + GFM, with TeX math, UI-850). */
  text: string;
  /** Opens a file path the text names (inline code or a relative link, UI-116); without it paths stay text. */
  onOpenPath?: OpenPath | undefined;
  /** The folder relative link targets and image sources are relative to (a Markdown file's own folder, `""` for the
   * root, UI-561); without it link targets are passed on as written. */
  linkBase?: string | undefined;
  /** Renders the images whose source is a relative file path (UI-562); without it they are plain `<img>`s. */
  image?: ComponentType<MarkdownImageProps> | undefined;
  /** `image` also renders the images whose source is an absolute file path (`/home/u/a.png`, UI-572), not a
   * root-relative URL: for text where no page's root makes sense, such as an agent's reply. */
  absoluteImages?: boolean | undefined;
  /** Renders the links to a thread (`omfp://thread/<id>`, UI-045), e.g. `ThreadLink` from `omfp/sdk`; without it
   * they are their text. */
  threadLink?: ComponentType<MarkdownThreadLinkProps> | undefined;
}

/**
 * Renders Markdown as prose (serif). Raw HTML in the source never becomes DOM: react-markdown without `rehype-raw`
 * drops it, and its URL filter removes `javascript:` links. Remote images and code-language classes are kept (UI-040).
 * The only markup that becomes DOM is a diagram's SVG, which Mermaid sanitizes (UI-982), and a formula's, which KaTeX
 * writes from the TeX with untrusted commands off (UI-852).
 */
export function Markdown({
  text,
  className,
  onOpenPath,
  linkBase,
  image,
  absoluteImages = false,
  threadLink,
  ...props
}: MarkdownProps) {
  const components = useMemo(() => {
    const paths = onOpenPath === undefined ? COMPONENTS : withPaths(onOpenPath, linkBase);
    const linked = threadLink === undefined ? paths : withThreads(paths, threadLink);
    return image === undefined ? linked : withImage(linked, image, linkBase ?? "", absoluteImages);
  }, [onOpenPath, linkBase, image, absoluteImages, threadLink]);
  return (
    <div {...props} className={cx("omfp-prose omfp-markdown", className)}>
      <ReactMarkdown remarkPlugins={REMARK_PLUGINS} components={components} urlTransform={urlTransform}>
        {text}
      </ReactMarkdown>
    </div>
  );
}

export interface CodeProps extends Omit<HTMLAttributes<HTMLElement>, "children"> {
  code: string;
  /** A `<pre>` block (keeps whitespace, scrolls horizontally) instead of inline code. */
  block?: boolean;
  /** Highlights a block as this language (a name, alias or file extension, UI-090); plain text when unknown. */
  language?: string | undefined;
}

/** Monospace code (UI-166), inline or as a block; a block names its language in its top-right corner
 * (`data-lang`, drawn by kit.css). */
export function Code({ code, block = false, language, className, ...props }: CodeProps) {
  if (block) {
    return (
      <pre {...props} className={cx("omfp-code omfp-code--block", className)} data-lang={language}>
        <code>
          <Highlighted code={code} language={language} />
        </code>
      </pre>
    );
  }
  return (
    <code {...props} className={cx("omfp-code", className)}>
      {code}
    </code>
  );
}
