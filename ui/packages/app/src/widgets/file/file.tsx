/**
 * `omfp:file { path, line?, diff? }` (UI-110..114): a workspace file in CodeMirror, read-only until `i` or Edit, then
 * edited with vim bindings and saved with `:w` / Ctrl-S against the hash the text was based on; a refused save shows
 * the file on disk against the user's text and writes nothing until the user decides. `diff` shows the whole file
 * against `HEAD`, unified or side by side. The text follows the file on disk while it has no unsaved change. Images
 * zoom, pan and compare with `HEAD` (image.tsx, UI-548..UI-555); video and audio files play in the browser's player
 * (media.tsx, UI-541); PDF documents show in the kit's viewer (pdf.tsx, UI-547). A file with a viewer (`viewers.tsx`:
 * Markdown, UI-560) shows it while read-only, with a toggle to its source. A file outside the workspace folder, opened
 * by its absolute path, is only read (UI-700).
 */

import type { FileContent } from "@omfp/protocol";
import {
  Button,
  CodeDiff,
  type CodeDiffLayout,
  CodeEditor,
  type CodeEditorHandle,
  EmptyState,
  type VimMode,
} from "omfp/kit";
import {
  EDITOR_MODE,
  type FileState,
  strings,
  useFile,
  useMissingFile,
  useReportError,
  useReportSuccess,
  useWindowState,
  type WidgetProps,
} from "omfp/sdk";
import { type ReactNode, type RefObject, useCallback, useEffect, useRef, useState } from "react";
import { ImageFile } from "./image.tsx";
import { MediaFile, mediaKind } from "./media.tsx";
import { PdfFile } from "./pdf.tsx";
import { extension, FileBar, type ViewerFailure, viewerOf } from "./viewers.tsx";

const text = strings.shell.file;

/** The version the editor's text is based on: what saves compare with (DMN-090). */
interface Base {
  readonly content: string;
  readonly hash: string;
}

type Mode = "view" | "edit" | "conflict";

export function FileWidget({ path, line, diff, jump }: WidgetProps): ReactNode {
  if (typeof path !== "string" || path === "") {
    return <EmptyState icon="file" title={text.noPath} className="omfp-file__empty" data-testid="file-none" />;
  }
  return (
    <FileView
      path={path}
      line={typeof line === "number" ? line : undefined}
      jump={typeof jump === "number" ? jump : undefined}
      diff={diff === true}
    />
  );
}

interface FileViewProps {
  readonly path: string;
  readonly line: number | undefined;
  /** Changes when the file is opened again, to jump to `line` again. */
  readonly jump: number | undefined;
  readonly diff: boolean;
}

function FileView({ path, line, jump, diff }: FileViewProps): ReactNode {
  const file = useFile(path);
  // A path that does not exist as given may name a file of the workspace (`theme.ts`, `src/theme.ts`, UI-117).
  const finding = useMissingFile(path, file.status === "missing", { line, diff });
  const root = useRef<HTMLElement>(null);
  return (
    <section
      ref={root}
      className="omfp-file omfp-focus-inset"
      data-testid="file"
      data-path={path}
      data-status={file.status}
      data-outside={file.file?.outside === true ? "true" : undefined}
      aria-label={text.label(path)}
      tabIndex={-1}
    >
      {file.status === "loading" ? (
        <p className="omfp-file__notice">{text.loading}</p>
      ) : file.status === "missing" ? (
        finding === "finding" || finding === "opened" ? (
          <p className="omfp-file__notice" data-testid="file-finding">
            {text.finding(path)}
          </p>
        ) : finding === "choosing" ? (
          <p className="omfp-file__notice" data-testid="file-choosing">
            {text.choosing(path)}
          </p>
        ) : (
          <p className="omfp-file__notice" data-testid="file-missing">
            {text.missing(path)}
          </p>
        )
      ) : file.file === undefined ? (
        <p className="omfp-file__notice" data-tone="error" data-testid="file-error">
          {text.failed(file.error ?? "")}
        </p>
      ) : extension(file.file.path) === "pdf" ? (
        <PdfFile file={file.file} />
      ) : file.file.kind === "text" && typeof file.file.content === "string" ? (
        <TextFile
          file={file.file}
          content={file.file.content}
          line={line}
          jump={jump}
          diff={diff}
          save={file.save}
          reload={file.reload}
          root={root}
        />
      ) : file.file.kind === "image" ? (
        <ImageFile file={file.file} diff={diff} />
      ) : (
        <OtherFile file={file.file} />
      )}
    </section>
  );
}

/** A video or audio file in a player, or another binary file named with its size. */
function OtherFile({ file }: { file: FileContent }): ReactNode {
  const media = file.kind === "binary" ? mediaKind(file.path) : undefined;
  if (media !== undefined) {
    return <MediaFile file={file} kind={media} />;
  }
  return (
    <EmptyState
      icon="file"
      title={text.binary(text.size(file.size))}
      className="omfp-file__empty"
      data-testid="file-binary"
    />
  );
}

interface TextFileProps {
  readonly file: FileContent;
  readonly content: string;
  readonly line: number | undefined;
  readonly jump: number | undefined;
  readonly diff: boolean;
  readonly save: FileState["save"];
  readonly reload: () => void;
  readonly root: RefObject<HTMLElement | null>;
}

function TextFile({ file, content, line, jump, diff, save, reload, root }: TextFileProps): ReactNode {
  const reportSuccess = useReportSuccess();
  const reportError = useReportError();
  const [, setEditorMode] = useWindowState(EDITOR_MODE);
  const editor = useRef<CodeEditorHandle | null>(null);
  const [mode, setMode] = useState<Mode>("view");
  const [showDiff, setShowDiff] = useState(diff);
  // A new `diff` prop (the file opened again) sets the toggle; an effect would also reset it each time a hidden tab
  // or view shows the file again (UI-403).
  const [diffProp, setDiffProp] = useState(diff);
  if (diffProp !== diff) {
    setDiffProp(diff);
    setShowDiff(diff);
  }
  const [layout, setLayout] = useState<CodeDiffLayout>("unified");
  const [dirty, setDirty] = useState(false);
  const [base, setBase] = useState<Base>({ content, hash: file.hash });
  // The user's text a refused save held (the conflict view's right side).
  const [mine, setMine] = useState<string | undefined>(undefined);
  const [notice, setNotice] = useState<string | undefined>(undefined);
  // A cut file, or one outside the workspace folder (the daemon refuses to write it, DMN-313), is only read (UI-700).
  const editable = file.truncated !== true && file.outside !== true;
  // A file with a viewer shows it while read-only, until the toggle shows the source (UI-560).
  const viewer = viewerOf(file.path);
  const [preview, setPreview] = useState(true);
  // A viewer that cannot show this version of the file hands it back to the source (UI-569).
  const [failure, setFailure] = useState<{ readonly hash: string; readonly failure: ViewerFailure } | undefined>();
  const failed = failure?.hash === file.hash ? failure.failure : undefined;
  const onViewerError = useCallback(
    (viewerFailure: ViewerFailure) => setFailure({ hash: file.hash, failure: viewerFailure }),
    [file.hash],
  );

  // The file on disk changed: follow it while nothing is unsaved (UI-110).
  useEffect(() => {
    if (!dirty && mode !== "conflict") {
      setBase((current) => (current.hash === file.hash ? current : { content, hash: file.hash }));
    }
  }, [content, file.hash, dirty, mode]);

  const leave = useCallback(() => root.current?.focus(), [root]);
  const onMode = useCallback((next: VimMode | undefined) => setEditorMode(next), [setEditorMode]);
  useEffect(() => () => setEditorMode(undefined), [setEditorMode]);

  const write = useCallback(
    async (textToSave: string, expectedHash: string) => {
      const result = await save(textToSave, expectedHash);
      if (result.ok) {
        setBase({ content: textToSave, hash: result.file.hash });
        setMine(undefined);
        setNotice(undefined);
        setMode("edit");
        reportSuccess(text.saved(result.file.path));
      } else if (result.conflict) {
        setMine(textToSave);
        setMode("conflict");
        reload();
      } else {
        reportError(new Error(text.saveFailed(result.error)));
      }
    },
    [save, reload, reportSuccess, reportError],
  );

  const onSave = useCallback((textToSave: string) => void write(textToSave, base.hash), [write, base.hash]);

  const quit = useCallback(
    (force: boolean) => {
      if (dirty && !force) {
        setNotice(text.unsaved);
        return;
      }
      if (dirty) {
        setBase({ content, hash: file.hash });
      }
      setNotice(undefined);
      setMode("view");
      leave();
    },
    [dirty, content, file.hash, leave],
  );

  const editing = mode !== "view";
  const previewing = mode === "view" && !showDiff && preview && viewer !== undefined && failed === undefined;
  const showEditor = mode !== "view" || (!showDiff && !previewing);
  return (
    <>
      <FileBar file={file}>
        <span className="omfp-file__actions">
          {mode === "view" && viewer !== undefined && (
            <Button
              size="sm"
              variant="quiet"
              aria-pressed={previewing}
              onPress={() => {
                // Over the diff, the toggle shows the viewer; else it switches between the viewer and the source. A
                // viewer that failed tries again.
                setPreview((on) => showDiff || failed !== undefined || !on);
                setShowDiff(false);
                setFailure(undefined);
              }}
              data-testid="file-view-toggle"
            >
              {viewer.label}
            </Button>
          )}
          {mode === "view" && (
            <Button
              size="sm"
              variant="quiet"
              aria-pressed={showDiff}
              onPress={() => setShowDiff((on) => !on)}
              data-testid="file-diff"
            >
              {text.diff}
            </Button>
          )}
          {mode === "view" && showDiff && (
            <Button
              size="sm"
              variant="quiet"
              onPress={() => setLayout((current) => (current === "unified" ? "split" : "unified"))}
              data-testid="file-layout"
            >
              {layout === "unified" ? text.split : text.unified}
            </Button>
          )}
          {mode === "view" && editable && (
            <Button size="sm" onPress={() => setMode("edit")} data-testid="file-edit">
              {text.edit}
            </Button>
          )}
          {mode === "edit" && (
            <>
              <Button
                size="sm"
                variant="primary"
                onPress={() => onSave(editor.current?.text() ?? base.content)}
                data-testid="file-save"
              >
                {text.save}
              </Button>
              <Button size="sm" onPress={() => quit(false)} data-testid="file-done">
                {text.done}
              </Button>
            </>
          )}
        </span>
      </FileBar>
      {file.truncated === true && <p className="omfp-file__notice">{text.truncated(text.size(file.size))}</p>}
      {file.outside === true && (
        <p className="omfp-file__notice" data-testid="file-outside">
          {text.outside}
        </p>
      )}
      {mode === "edit" && dirty && file.hash !== base.hash && (
        <p className="omfp-file__notice" data-tone="warning" data-testid="file-changed">
          {text.changedOnDisk}
        </p>
      )}
      {notice !== undefined && (
        <p className="omfp-file__notice" data-tone="warning" data-testid="file-notice">
          {notice}
        </p>
      )}
      {mode === "view" && failed !== undefined && (
        <p className="omfp-file__notice" data-tone="error" data-testid="file-viewer-error">
          {failed.message}
        </p>
      )}
      {mode === "conflict" && mine !== undefined && (
        <Conflict
          path={file.path}
          disk={content}
          mine={mine}
          onKeep={() => {
            setBase({ content: mine, hash: file.hash });
            setMine(undefined);
            setMode("edit");
          }}
          onOverwrite={() => void write(mine, file.hash)}
          onDiscard={() => {
            setBase({ content, hash: file.hash });
            setMine(undefined);
            setMode("view");
          }}
        />
      )}
      {mode === "view" && showDiff && <HeadDiff file={file} content={content} layout={layout} />}
      {previewing && (
        <div className="omfp-file__viewer" data-testid="file-viewer" data-viewer={viewer.name}>
          <viewer.View file={file} content={content} onError={onViewerError} />
        </div>
      )}
      {showEditor && (
        <CodeEditor
          className="omfp-file__editor"
          hidden={mode === "conflict"}
          value={base.content}
          path={file.path}
          readOnly={!editing}
          line={failed?.line ?? line}
          jump={jump}
          handle={editor}
          aria-label={text.editorLabel(file.path)}
          data-testid="file-editor"
          data-mode={mode}
          data-dirty={dirty}
          loading={<p className="omfp-file__notice">{text.loadingEditor}</p>}
          onSave={onSave}
          onQuit={quit}
          onLeave={leave}
          onRequestEdit={editable ? () => setMode("edit") : undefined}
          onModeChange={onMode}
          onDirtyChange={setDirty}
        />
      )}
    </>
  );
}

/** The whole file against its `HEAD` text (UI-114). */
function HeadDiff({
  file,
  content,
  layout,
}: {
  file: FileContent;
  content: string;
  layout: CodeDiffLayout;
}): ReactNode {
  const outside = typeof file.repo !== "string";
  return (
    <>
      {(outside || typeof file.head !== "string") && (
        <p className="omfp-file__notice" data-testid="file-head-note">
          {outside ? text.noRepository : text.newFile}
        </p>
      )}
      <CodeDiff
        className="omfp-file__diff"
        original={typeof file.head === "string" ? file.head : ""}
        value={content}
        path={file.path}
        layout={layout}
        aria-label={text.diffLabel(file.path)}
        data-testid="file-head-diff"
        loading={<p className="omfp-file__notice">{text.loadingEditor}</p>}
      />
    </>
  );
}

interface ConflictProps {
  readonly path: string;
  readonly disk: string;
  readonly mine: string;
  onKeep(): void;
  onOverwrite(): void;
  onDiscard(): void;
}

/** A refused save (UI-113): the file on disk against the user's text, and the user's choice. */
function Conflict({ path, disk, mine, onKeep, onOverwrite, onDiscard }: ConflictProps): ReactNode {
  return (
    <div className="omfp-file__conflict" data-testid="file-conflict">
      <p className="omfp-file__notice" data-tone="error" role="alert">
        {text.conflict}
      </p>
      <div className="omfp-file__actions">
        <Button size="sm" onPress={onKeep} data-testid="file-keep">
          {text.keepEditing}
        </Button>
        <Button size="sm" variant="danger" onPress={onOverwrite} data-testid="file-overwrite">
          {text.overwrite}
        </Button>
        <Button size="sm" onPress={onDiscard} data-testid="file-discard">
          {text.discard}
        </Button>
      </div>
      <CodeDiff
        className="omfp-file__diff"
        original={disk}
        value={mine}
        path={path}
        layout="split"
        aria-label={text.conflictLabel(path)}
        data-testid="file-conflict-diff"
        loading={<p className="omfp-file__notice">{text.loadingEditor}</p>}
      />
    </div>
  );
}
