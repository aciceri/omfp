/** L1: what a window opens on, from its address bar (`omfp desktop --thread/--workspace/--view/--prompt`). */

import { describe, expect, test } from "bun:test";
import { findWorkspace, takeLaunch } from "../../src/vm/launch.ts";
import { workspace } from "../support/fixtures.ts";

function pages() {
  const replaced: string[] = [];
  const page = (hash: string) => ({
    location: { hash, pathname: "/", search: "?x=1" },
    history: {
      replaceState: (_data: unknown, _unused: string, url?: string | URL | null) => replaced.push(String(url)),
    },
  });
  return { replaced, page };
}

describe("launch", () => {
  // spec: UI-218
  test("`thread=<id>` selects a thread once: it is read and removed from the address bar", () => {
    const { replaced, page } = pages();
    expect(takeLaunch(page("#local=1&thread=t1"))).toEqual({ thread: "t1" });
    expect(replaced).toEqual(["/?x=1#local=1"]);
    expect(takeLaunch(page("#thread=a%26b%3Dc"))).toEqual({ thread: "a&b=c" });
    expect(replaced[1]).toBe("/?x=1");
    // Nothing to take: the address stays.
    expect(takeLaunch(page("#local=1"))).toBeUndefined();
    expect(takeLaunch(page("#threads=t1"))).toBeUndefined();
    expect(replaced).toHaveLength(2);
    // Empty or malformed: removed, nothing selected.
    expect(takeLaunch(page("#thread="))).toEqual({});
    expect(takeLaunch(page("#thread=%E0%A4%A"))).toEqual({});
    expect(replaced).toHaveLength(4);
  });

  // spec: UI-214
  test("`workspace`, `view` and `prompt` are read once; the prompt keeps its spaces and lines", () => {
    const { replaced, page } = pages();
    expect(
      takeLaunch(page("#local=1&workspace=Home&view=web&prompt=S%C3%AC%2C%20due%0Arighe%20%26%20pi%C3%B9%20")),
    ).toEqual({ workspace: "Home", view: "web", prompt: "Sì, due\nrighe & più " });
    expect(replaced).toEqual(["/?x=1#local=1"]);
    expect(takeLaunch(page("#workspace=%20&view=%20web%20&prompt=%20%0A"))).toEqual({ view: "web" });
  });

  // spec: UI-214
  test("a workspace is found by id, name, name in any case when only one matches, or folder", () => {
    const home = { ...workspace("w1"), name: "Home", path: "/srv/home" };
    const other = { ...workspace("w2"), name: "home2", path: "/srv/other" };
    const all = [home, other];
    expect(findWorkspace(all, "w2")?.id).toBe("w2");
    expect(findWorkspace(all, "Home")?.id).toBe("w1");
    expect(findWorkspace(all, "home")?.id).toBe("w1");
    expect(findWorkspace(all, "/srv/other/")?.id).toBe("w2");
    expect(findWorkspace(all, "nope")).toBeUndefined();
    // An exact name wins over one differing in case; two in another case are ambiguous.
    const twins = [home, { ...workspace("w3"), name: "HOME" }];
    expect(findWorkspace(twins, "HOME")?.id).toBe("w3");
    expect(findWorkspace(twins, "home")).toBeUndefined();
  });
});
