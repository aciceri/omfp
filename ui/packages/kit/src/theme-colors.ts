/**
 * Reads `--omfp-` tokens through a hidden probe, normalizing browser-only color notation through a 1×1 canvas. The
 * probe goes in `within` (default `<body>`), so tokens a subtree overrides are read as that subtree sees them.
 */
export function cssColorReader<T>(
  document: Document,
  parse: (color: string) => T | undefined,
  within: Element = document.body,
): (token: string) => T | undefined {
  let context: CanvasRenderingContext2D | null | undefined;
  return (token) => {
    const probe = document.createElement("span");
    probe.hidden = true;
    probe.style.color = `var(--omfp-${token})`;
    within.append(probe);
    const computed = document.defaultView?.getComputedStyle(probe).color ?? "";
    probe.remove();
    const parsed = parse(computed);
    if (parsed !== undefined || computed === "") {
      return parsed;
    }
    if (context === undefined) {
      const canvas = document.createElement("canvas");
      canvas.width = 1;
      canvas.height = 1;
      context = canvas.getContext("2d", { willReadFrequently: true });
    }
    if (context === null) {
      return undefined;
    }
    context.clearRect(0, 0, 1, 1);
    context.fillStyle = computed;
    context.fillRect(0, 0, 1, 1);
    const [red, green, blue] = context.getImageData(0, 0, 1, 1).data;
    if (red === undefined || green === undefined || blue === undefined) {
      return undefined;
    }
    return parse(
      `#${red.toString(16).padStart(2, "0")}${green.toString(16).padStart(2, "0")}${blue.toString(16).padStart(2, "0")}`,
    );
  };
}
