/**
 * Images in the kit (UI-549..UI-555): `ImageView` shows an image fitted to the viewer or zoomed — buttons, the wheel
 * with Ctrl (a trackpad pinch), two fingers on a touch screen — and panned by dragging, and compares it with an earlier
 * version side by side or under a slider, both at the same scale and scrolled together. `ImageDialog` opens one in a
 * modal dialog as large as the window (UI-474).
 */
import {
  type CSSProperties,
  type HTMLAttributes,
  type ReactNode,
  type RefObject,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { Button, IconButton } from "./button.tsx";
import { cx } from "./class.ts";
import { Dialog } from "./dialog.tsx";
import { Icon } from "./icons.tsx";
import {
  anchoredScroll,
  clampZoom,
  containScale,
  IMAGE_ZOOM_STEPS,
  type Size,
  wheelZoom,
  zoomIn,
  zoomLabel,
  zoomOut,
} from "./zoom.ts";

/** What the viewer says, in the window's language. */
export interface ImageViewLabels {
  readonly zoomIn: string;
  readonly zoomOut: string;
  /** The toggle that fits the whole image in the viewer (the default). */
  readonly fit: string;
  /** Zoom to the image's own pixels: `100%`. */
  readonly actualSize: string;
  /** The viewer's toolbar. */
  readonly toolbar: string;
  /** An image the browser cannot show (not an image, damaged, unreachable). */
  readonly failed: string;
  /** The earlier version's name, e.g. `HEAD`. */
  readonly before: string;
  /** The shown version's name, e.g. `Working copy`. */
  readonly after: string;
  /** The slider between the versions. */
  readonly slider: string;
}

/** How two versions are compared: next to each other, or one over the other cut by a slider. */
export type ImageCompare = "split" | "slider";

type Side = "before" | "after";

type RootAttributes = Omit<HTMLAttributes<HTMLDivElement>, "children">;

export interface ImageViewProps extends RootAttributes {
  /** The image: same-origin or `data:`. */
  readonly src: string;
  readonly alt: string;
  /** An earlier version to compare with, shown when `compare` is given. */
  readonly before?: { readonly src: string; readonly alt: string } | undefined;
  readonly compare?: ImageCompare | undefined;
  readonly labels: ImageViewLabels;
}

/** The size browsers give an image without one of its own (an SVG without `width`/`height`). */
const DEFAULT_SIZE: Size = { width: 300, height: 150 };

/** A position in a viewport, in CSS pixels from its top left corner (inside its border). */
interface Point {
  readonly x: number;
  readonly y: number;
}

/** What the gestures need of the viewer, read when they happen. */
interface Gestures {
  readonly scale: number;
  zoomTo(next: number, viewport: HTMLElement, point: Point): void;
}

/** The size inside `element`'s padding, live. */
function useViewSize(element: HTMLElement | null): Size {
  const [size, setSize] = useState<Size>({ width: 0, height: 0 });
  useLayoutEffect(() => {
    if (element === null) {
      return;
    }
    const measure = () => setSize({ width: element.clientWidth, height: element.clientHeight });
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);
  return size;
}

/**
 * Wheel with Ctrl (or a trackpad pinch, which browsers send as such) zooms around the pointer; one pointer dragging
 * pans; two pointers pinching zoom around their middle. The wheel listener is not passive, so the page never zooms.
 */
function attachGestures(viewport: HTMLElement, gestures: RefObject<Gestures>): () => void {
  const down = new Map<number, Point>();
  let pinch: { distance: number; scale: number } | undefined;
  const local = (event: PointerEvent | WheelEvent): Point => {
    const box = viewport.getBoundingClientRect();
    return { x: event.clientX - box.left - viewport.clientLeft, y: event.clientY - box.top - viewport.clientTop };
  };
  const pair = (): [Point, Point] | undefined => {
    const [a, b] = [...down.values()];
    return a !== undefined && b !== undefined ? [a, b] : undefined;
  };
  const listeners = {
    wheel: (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey) {
        event.preventDefault();
        gestures.current.zoomTo(wheelZoom(gestures.current.scale, event.deltaY), viewport, local(event));
      }
    },
    pointerdown: (event: PointerEvent) => {
      if (event.button !== 0) {
        return;
      }
      down.set(event.pointerId, local(event));
      try {
        viewport.setPointerCapture?.(event.pointerId);
      } catch {
        // A pointer already released cannot be captured: the drag then follows it only over the viewport.
      }
      const two = pair();
      pinch = two && { distance: Math.hypot(two[0].x - two[1].x, two[0].y - two[1].y), scale: gestures.current.scale };
      viewport.dataset.dragging = "true";
    },
    pointermove: (event: PointerEvent) => {
      const last = down.get(event.pointerId);
      if (last === undefined) {
        return;
      }
      const now = local(event);
      down.set(event.pointerId, now);
      const two = pair();
      if (pinch !== undefined && two !== undefined && pinch.distance > 0) {
        const [a, b] = two;
        const middle = { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 };
        gestures.current.zoomTo((pinch.scale * Math.hypot(a.x - b.x, a.y - b.y)) / pinch.distance, viewport, middle);
      } else {
        viewport.scrollLeft -= now.x - last.x;
        viewport.scrollTop -= now.y - last.y;
      }
    },
    pointerup: (event: PointerEvent) => {
      down.delete(event.pointerId);
      pinch = undefined;
      if (down.size === 0) {
        delete viewport.dataset.dragging;
      }
    },
  };
  viewport.addEventListener("wheel", listeners.wheel, { passive: false });
  viewport.addEventListener("pointerdown", listeners.pointerdown);
  viewport.addEventListener("pointermove", listeners.pointermove);
  viewport.addEventListener("pointerup", listeners.pointerup);
  viewport.addEventListener("pointercancel", listeners.pointerup);
  return () => {
    viewport.removeEventListener("wheel", listeners.wheel);
    viewport.removeEventListener("pointerdown", listeners.pointerdown);
    viewport.removeEventListener("pointermove", listeners.pointermove);
    viewport.removeEventListener("pointerup", listeners.pointerup);
    viewport.removeEventListener("pointercancel", listeners.pointerup);
  };
}

/** The box both versions fit in, at 100%: the larger width and height of those known. */
function unionBox(sizes: readonly (Size | undefined)[]): Size {
  let box: Size = { width: 0, height: 0 };
  for (const size of sizes) {
    if (size !== undefined) {
      box = { width: Math.max(box.width, size.width), height: Math.max(box.height, size.height) };
    }
  }
  return box;
}

/**
 * Where an image `own` large at 100% goes in the versions' `box` shown at `scale` (CSS centres the box in the
 * viewport while it is smaller): centred in the box; `clip` cuts it left of that x of the box.
 */
export function placeImage(box: Size, own: Size, scale: number, clip?: number): CSSProperties {
  const left = ((box.width - own.width) * scale) / 2;
  const top = ((box.height - own.height) * scale) / 2;
  return {
    left,
    top,
    inlineSize: own.width * scale,
    blockSize: own.height * scale,
    ...(clip === undefined ? {} : { clipPath: `inset(0 0 0 ${Math.max(0, clip - left)}px)` }),
  };
}

/** One version of the image, or why it cannot show. */
function Picture(props: {
  side: Side;
  src: string;
  alt: string;
  style: CSSProperties;
  failed: boolean;
  loaded: boolean;
  failedLabel: string;
  onLoad: (size: Size) => void;
  onError: () => void;
}): ReactNode {
  if (props.failed) {
    return (
      <p className="omfp-image__notice" data-tone="error" data-testid="image-error" data-side={props.side}>
        {props.failedLabel}
      </p>
    );
  }
  return (
    <img
      src={props.src}
      alt={props.alt}
      draggable={false}
      className="omfp-image__img"
      style={props.style}
      data-testid={`image-${props.side}`}
      data-loaded={props.loaded}
      onLoad={({ currentTarget: image }) =>
        props.onLoad(
          image.naturalWidth > 0 && image.naturalHeight > 0
            ? { width: image.naturalWidth, height: image.naturalHeight }
            : DEFAULT_SIZE,
        )
      }
      onError={props.onError}
    />
  );
}

/** The zoom of a viewer over `box` (at 100%) in viewports of `view`'s size: fitted, or a scale zoomed around a point. */
function useZoom(viewports: RefObject<(HTMLElement | null)[]>, view: Size, box: Size, layout: unknown) {
  const [zoom, setZoom] = useState({ fit: true, scale: 1 });
  const scale = zoom.fit ? containScale(view, box) : zoom.scale;
  // The scroll a zoom asked for, applied once the new scale is laid out.
  const pending = useRef<{ left: number; top: number } | undefined>(undefined);
  const zoomTo = (next: number, viewport = viewports.current[0] ?? null, point?: Point) => {
    const clamped = clampZoom(next, IMAGE_ZOOM_STEPS);
    if (viewport !== null && clamped !== scale) {
      const at = point ?? { x: viewport.clientWidth / 2, y: viewport.clientHeight / 2 };
      pending.current = {
        left: anchoredScroll(viewport.scrollLeft, at.x, viewport.clientWidth, box.width, scale, clamped),
        top: anchoredScroll(viewport.scrollTop, at.y, viewport.clientHeight, box.height, scale, clamped),
      };
    }
    setZoom({ fit: false, scale: clamped });
  };
  // biome-ignore lint/correctness/useExhaustiveDependencies: the scroll a zoom asked for applies once its scale is laid out.
  useLayoutEffect(() => {
    const scroll = pending.current;
    pending.current = undefined;
    for (const viewport of scroll === undefined ? [] : viewports.current) {
      viewport?.scrollTo({ left: scroll?.left ?? 0, top: scroll?.top ?? 0 });
    }
  }, [viewports, scale]);
  const gestures = useRef<Gestures>({ scale, zoomTo });
  gestures.current = { scale, zoomTo };
  // biome-ignore lint/correctness/useExhaustiveDependencies: the viewports change with the comparison's layout.
  useEffect(() => {
    const detach = viewports.current.flatMap((viewport) => (viewport ? [attachGestures(viewport, gestures)] : []));
    return () => {
      for (const cleanup of detach) {
        cleanup();
      }
    };
  }, [viewports, layout]);
  return { scale, fit: zoom.fit, zoomTo, fitAgain: () => setZoom((now) => ({ ...now, fit: true })) };
}

/**
 * An image fitted to the viewer or zoomed and panned (UI-549..UI-551), alone or compared with `before` side by side
 * or under a slider (UI-553..UI-555). It fills the height its container gives it.
 */
export function ImageView({ src, alt, before, compare, labels, className, ...props }: ImageViewProps): ReactNode {
  const mode = before === undefined ? undefined : compare;
  const viewports = useRef<(HTMLElement | null)[]>([]);
  const [first, setFirst] = useState<HTMLElement | null>(null);
  // One stable ref per pane: a new callback each render would detach and attach the viewport every time.
  const [paneRefs] = useState(() =>
    [0, 1].map((index) => (element: HTMLElement | null) => {
      viewports.current[index] = element;
      if (index === 0) {
        setFirst(element);
      }
    }),
  );
  const view = useViewSize(first);
  const [naturals, setNaturals] = useState<Readonly<Record<string, Size>>>({});
  const [failed, setFailed] = useState<readonly string[]>([]);
  const [slider, setSlider] = useState(50);
  const versions =
    mode === undefined || before === undefined
      ? [{ side: "after" as const, src, alt }]
      : [
          { side: "before" as const, ...before },
          { side: "after" as const, src, alt },
        ];
  const box = unionBox(versions.map((version) => naturals[version.src]));
  const { scale, fit, zoomTo, fitAgain } = useZoom(viewports, view, box, `${mode}:${first === null}`);
  const divider = (box.width * scale * slider) / 100;
  const picture = (version: (typeof versions)[number], clip?: number) => (
    <Picture
      key={version.side}
      side={version.side}
      src={version.src}
      alt={version.alt}
      style={placeImage(box, naturals[version.src] ?? box, scale, clip)}
      failed={failed.includes(version.src)}
      loaded={naturals[version.src] !== undefined}
      failedLabel={labels.failed}
      onLoad={(size) => setNaturals((known) => ({ ...known, [version.src]: size }))}
      onError={() => setFailed((sources) => [...sources, version.src])}
    />
  );
  const panes =
    mode === "slider"
      ? [
          {
            side: "both",
            label: `${labels.before} | ${labels.after}`,
            captions: [labels.before, labels.after],
            layers: versions.map((version) => picture(version, version.side === "after" ? divider : undefined)),
          },
        ]
      : versions.map((version) => ({
          side: version.side,
          label: mode === undefined ? alt : labels[version.side],
          captions: mode === undefined ? [] : [labels[version.side]],
          layers: [picture(version)],
        }));
  const stage = { inlineSize: box.width * scale, blockSize: box.height * scale };
  return (
    <div data-testid="image-view" {...props} className={cx("omfp-image", className)} data-compare={mode ?? "none"}>
      <ZoomBar
        labels={labels}
        scale={scale}
        fit={fit}
        onZoom={(next) => zoomTo(next)}
        onFit={fitAgain}
        slider={mode === "slider" ? { value: slider, onChange: setSlider } : undefined}
      />
      <div className="omfp-image__panes" data-compare={mode ?? "none"}>
        {panes.map((pane, index) => (
          <Pane
            key={pane.side}
            {...pane}
            viewport={paneRefs[index]}
            onScroll={(source) => syncScroll(viewports.current, source)}
            stage={stage}
            divider={mode === "slider" ? divider : undefined}
          />
        ))}
      </div>
    </div>
  );
}

export interface ImageDialogProps {
  /** The image the dialog shows; `undefined` keeps it closed. */
  readonly image: { readonly src: string; readonly alt: string } | undefined;
  /** The dialog's heading. */
  readonly title: string;
  readonly labels: ImageViewLabels;
  /** Escape or a click outside closed it. */
  readonly onClose: () => void;
  readonly "data-testid"?: string | undefined;
}

/**
 * An image at full size (UI-474): a modal dialog as large as the window, titled `title`, whose `ImageView` opens fitted
 * and zooms and pans like any (UI-549..UI-551); Escape or a click outside closes it, the focus going back to what
 * opened it.
 */
export function ImageDialog({ image, title, labels, onClose, "data-testid": testId }: ImageDialogProps): ReactNode {
  return (
    <Dialog
      isOpen={image !== undefined}
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
      isDismissable
      title={title}
      className="omfp-image-dialog"
      data-testid={testId}
    >
      {image !== undefined && (
        <ImageView src={image.src} alt={image.alt} labels={labels} className="omfp-image-dialog__view" />
      )}
    </Dialog>
  );
}

interface PaneProps {
  readonly side: string;
  readonly label: string;
  /** Visible names of what the pane shows, left to right (none for a single image). */
  readonly captions: readonly string[];
  readonly layers: ReactNode;
  readonly viewport: ((element: HTMLElement | null) => void) | undefined;
  readonly onScroll: (viewport: HTMLElement) => void;
  readonly stage: CSSProperties;
  /** Where the slider cuts, in the stage's pixels. */
  readonly divider: number | undefined;
}

/** One scrolling viewport of the viewer, with its captions. */
function Pane({ side, label, captions, layers, viewport, onScroll, stage, divider }: PaneProps): ReactNode {
  return (
    <div className="omfp-image__pane">
      {captions.length > 0 && (
        <div className="omfp-image__captions" aria-hidden="true">
          {captions.map((caption) => (
            <span key={caption}>{caption}</span>
          ))}
        </div>
      )}
      <section
        ref={viewport}
        className="omfp-image__viewport omfp-focus-inset"
        aria-label={label}
        // biome-ignore lint/a11y/noNoninteractiveTabindex: a scrolling region the keyboard must reach to scroll (UI-092).
        tabIndex={0}
        onScroll={(event) => onScroll(event.currentTarget)}
        data-testid="image-viewport"
        data-side={side}
      >
        <div className="omfp-image__stage">
          <div className="omfp-image__box" style={stage}>
            {layers}
            {divider !== undefined && (
              <div className="omfp-image__divider" style={{ left: divider }} data-testid="image-divider" />
            )}
          </div>
        </div>
      </section>
    </div>
  );
}

/** Scrolls the other viewports of a side-by-side comparison where `source` is. */
function syncScroll(viewports: readonly (HTMLElement | null)[], source: HTMLElement): void {
  for (const viewport of viewports) {
    if (
      viewport !== null &&
      viewport !== source &&
      (viewport.scrollLeft !== source.scrollLeft || viewport.scrollTop !== source.scrollTop)
    ) {
      viewport.scrollTo({ left: source.scrollLeft, top: source.scrollTop });
    }
  }
}

interface ZoomBarProps {
  readonly labels: ImageViewLabels;
  readonly scale: number;
  readonly fit: boolean;
  readonly onZoom: (scale: number) => void;
  readonly onFit: () => void;
  readonly slider: { readonly value: number; readonly onChange: (value: number) => void } | undefined;
}

function ZoomBar({ labels, scale, fit, onZoom, onFit, slider }: ZoomBarProps): ReactNode {
  return (
    <div className="omfp-image__bar" role="toolbar" aria-label={labels.toolbar}>
      <IconButton
        label={labels.zoomOut}
        icon={<Icon name="minus" />}
        onPress={() => onZoom(zoomOut(scale, IMAGE_ZOOM_STEPS))}
        data-testid="image-zoom-out"
      />
      <span className="omfp-image__zoom" data-testid="image-zoom">
        {zoomLabel(scale)}
      </span>
      <IconButton
        label={labels.zoomIn}
        icon={<Icon name="plus" />}
        onPress={() => onZoom(zoomIn(scale, IMAGE_ZOOM_STEPS))}
        data-testid="image-zoom-in"
      />
      <Button size="sm" variant="quiet" aria-pressed={fit} onPress={onFit} data-testid="image-fit">
        {labels.fit}
      </Button>
      <Button size="sm" variant="quiet" onPress={() => onZoom(1)} data-testid="image-actual">
        {labels.actualSize}
      </Button>
      {slider !== undefined && (
        <input
          type="range"
          className="omfp-image__slider"
          min={0}
          max={100}
          value={slider.value}
          aria-label={labels.slider}
          onChange={(event) => slider.onChange(Number(event.currentTarget.value))}
          data-testid="image-slider"
        />
      )}
    </div>
  );
}
