const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * Calls `changed` whenever the effective theme may have changed: an attribute of `<html>` (only `attributes`, when
 * given), the system's `prefers-color-scheme`, any change under `<head>` (the user theme's `<style>` text or its
 * replacement, UI-091) and any stylesheet finishing loading. For what reads the tokens from script: the contrast guard
 * (UI-196), Mermaid diagrams (UI-981). Returns the unsubscribe.
 */
export function watchTheme(document: Document, changed: () => void, attributes?: readonly string[]): () => void {
  const observer = new MutationObserver(changed);
  observer.observe(
    document.documentElement,
    attributes === undefined ? { attributes: true } : { attributes: true, attributeFilter: [...attributes] },
  );
  observer.observe(document.head, { childList: true, subtree: true, characterData: true });
  const loaded = (event: Event) => {
    if (event.target instanceof HTMLLinkElement) {
      changed();
    }
  };
  document.addEventListener("load", loaded, true);
  const dark = document.defaultView?.matchMedia(DARK_QUERY);
  dark?.addEventListener("change", changed);
  return () => {
    observer.disconnect();
    document.removeEventListener("load", loaded, true);
    dark?.removeEventListener("change", changed);
  };
}
