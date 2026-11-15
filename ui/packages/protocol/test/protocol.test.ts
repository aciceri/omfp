import { describe, expect, test } from "bun:test";
import {
  formatTopic,
  INDEX_TOPIC,
  jobTopic,
  METHODS,
  type ParamsOf,
  parseTopic,
  terminalTopic,
  threadTopic,
  viewsTopic,
  workspaceTopic,
} from "../src/index.ts";

describe("topics", () => {
  // spec: PRT-040, PRT-041, PRT-135
  test("round trip through their names", () => {
    const cases = [
      [{ kind: "index" }, "index"],
      [{ kind: "thread", id: "01J0THREAD" }, "thread/01J0THREAD"],
      [{ kind: "workspace", id: "01J0WS" }, "workspace/01J0WS"],
      [{ kind: "views", id: "01J0WS" }, "views/01J0WS"],
      [{ kind: "job", id: "01J0JOB" }, "job/01J0JOB"],
      [{ kind: "terminal", id: "01J0TERMINAL" }, "terminal/01J0TERMINAL"],
    ] as const;
    for (const [topic, name] of cases) {
      expect(formatTopic(topic)).toBe(name);
      expect(parseTopic(name)).toEqual(topic);
    }
    expect(INDEX_TOPIC).toBe("index");
    expect(threadTopic("01J0THREAD")).toBe("thread/01J0THREAD");
    expect(workspaceTopic("01J0WS")).toBe("workspace/01J0WS");
    expect(viewsTopic("01J0WS")).toBe("views/01J0WS");
    expect(jobTopic("01J0JOB")).toBe("job/01J0JOB");
    expect(terminalTopic("01J0TERMINAL")).toBe("terminal/01J0TERMINAL");
  });

  // spec: PRT-135
  test("malformed names are rejected", () => {
    const names = [
      "",
      "index/",
      "index/1",
      "thread",
      "thread/",
      "thread/a/b",
      "thread/a b",
      "Thread/01J",
      "threads/01J",
      "repo/01J",
      "terminal",
      "/01J",
      " index",
    ];
    for (const name of names) {
      expect(parseTopic(name)).toBeUndefined();
    }
  });
});

describe("method table", () => {
  test("lists every method once and types the params", () => {
    expect(new Set(METHODS).size).toBe(METHODS.length);
    // @ts-expect-error: `text` is required by `thread.prompt`.
    const missing: ParamsOf<"thread.prompt"> = { threadId: "01J" };
    void missing;
  });
});
