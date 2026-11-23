import { type ComponentProps, type ComponentType, type HTMLAttributes, useMemo } from "react";
import ReactMarkdown, { type Components, type ExtraProps } from "react-markdown";
import remarkGfm from "remark-gfm";
import { cx } from "./class.ts";
import { Highlighted } from "./highlight.tsx";
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

const REMARK_PLUGINS = [remarkGfm, remarkClosedMermaid];

const FENCE_LANGUAGE = /(?:^|\s)language-(\S+)/;

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

function remarkClosedMermaid(this: {
  data(key: string): unknown;
  data(key: string, value: unknown): unknown;
}) {
  const extension = {
    enter: {
      codeFencedFence(this: FenceContext) {
        // This hook runs for both fences; the default exit handler starts buffering after the opening one.
        if (!this.data.flowCodeInside) return;
        const code = this.stack.findLast((node) => node.type === "code");
        if (code?.lang?.toLowerCase() !== "mermaid") return;
        code.data ??= {};
        code.data.hProperties ??= {};
        code.data.hProperties["data-mermaid-closed"] = true;
      },
    },
  };
  const extensions = this.data("fromMarkdownExtensions");
  if (Array.isArray(extensions)) extensions.push(extension);
  else this.data("fromMarkdownExtensions", [extension]);
}

/** A fenced code block, highlighted (UI-090); a closed `mermaid` block is drawn instead (UI-981). */
function FencedBlock({ node, className, ...props }: ComponentProps<"pre"> & ExtraProps) {
  const language = fenceLanguage(node);
  const code = node?.children[0];
  if (
    language?.toLowerCase() === "mermaid" &&
    code?.type === "element" &&
    code.properties["data-mermaid-closed"] === true
  ) {
    const source = code.children.map((child) => (child.type === "text" ? child.value : "")).join("");
    return <MermaidDiagram source={source.replace(/\n$/, "")} />;
  }
  return <pre {...props} className={cx("omfp-code omfp-code--block", className)} data-lang={language} />;
}

/** Links open in a new tab without leaking the referrer; fenced code with a language is highlighted (UI-090), a closed
 * `mermaid` fence drawn (UI-981); a table sits in a region that scrolls sideways when it is wider than the text
 * (UI-369), focusable so the keyboard can scroll it; everything else keeps react-markdown's elements. */
const COMPONENTS = {
  a: ({ node: _node, ...props }) => <a {...props} target="_blank" rel="noreferrer" />,
  pre: FencedBlock,
  table: ({ node: _node, ...props }) => (
    // biome-ignore lint/a11y/noNoninteractiveTabindex: a scrollable region must be focusable to scroll by keyboard.
    <section className="omfp-markdown__table" aria-label="Table" tabIndex={0}>
      <table {...props} />
    </section>
  ),
  code: ({ node: _node, className, children, ...props }) => {
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

/** {@link COMPONENTS} where file paths open with `onOpen` (UI-116): inline code that reads as a path, and links to a
 * relative path (resolved against `linkBase` when given, UI-561). A fenced block's text ends with a line break, so it
 * never counts as inline code. */
function withPaths(onOpen: OpenPath, linkBase: string | undefined): Components {
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
  /** Markdown source (CommonMark + GFM). */
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
}

/**
 * Renders Markdown as prose (serif). Raw HTML in the source never becomes DOM: react-markdown without `rehype-raw`
 * drops it, and its URL filter removes `javascript:` links. Remote images and code-language classes are kept (UI-040).
 * The only markup that becomes DOM is a diagram's SVG, which Mermaid sanitizes (UI-982).
 */
export function Markdown({
  text,
  className,
  onOpenPath,
  linkBase,
  image,
  absoluteImages = false,
  ...props
}: MarkdownProps) {
  const components = useMemo(() => {
    const linked = onOpenPath === undefined ? COMPONENTS : withPaths(onOpenPath, linkBase);
    return image === undefined ? linked : withImage(linked, image, linkBase ?? "", absoluteImages);
  }, [onOpenPath, linkBase, image, absoluteImages]);
  return (
    <div {...props} className={cx("omfp-prose omfp-markdown", className)}>
      <ReactMarkdown remarkPlugins={REMARK_PLUGINS} components={components}>
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
