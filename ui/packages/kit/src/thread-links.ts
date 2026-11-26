/**
 * Links to a thread in text (UI-045): `omfp://thread/<id>`, the one form agents (AGT-045), widgets and users write.
 * Which thread it names and what a click does is the app's business (`ThreadLink` in `omfp/sdk`); this only tells such
 * a link from other links.
 */

/** The scheme and path every thread link starts with. */
const PREFIX = "omfp://thread/";

/** The thread a link's target names (`omfp://thread/<id>`, with or without a trailing `/`, query or fragment), its id
 * percent-decoded; `undefined` for every other target. */
export function threadLinkId(href: string | undefined): string | undefined {
  if (href === undefined || href.slice(0, PREFIX.length).toLowerCase() !== PREFIX) {
    return undefined;
  }
  const [segment = ""] = href.slice(PREFIX.length).split(/[/?#]/, 1);
  let id: string;
  try {
    id = decodeURIComponent(segment).trim();
  } catch {
    return undefined;
  }
  return id === "" ? undefined : id;
}

/** The link to thread `threadId` (`omfp://thread/<id>`), for text the kit's `Markdown` renders. */
export function threadLinkUrl(threadId: string): string {
  return `${PREFIX}${encodeURIComponent(threadId)}`;
}
