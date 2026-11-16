/** L4: `useJobs` (AGT-090) over a fake client: the workspace's `jobs/<id>` and the actions on its jobs. */

import { afterEach, describe, expect, test } from "bun:test";
import { CallError } from "@omfp/client";
import { jobsTopic } from "@omfp/protocol";
import { act, cleanup, screen } from "@testing-library/react";
import { type Jobs, useJobs, type WidgetInfoScope, WidgetScope } from "../../src/sdk/index.ts";
import { jobView, runInfo } from "../support/fixtures.ts";
import { released, renderWithHost } from "../support/host.tsx";

afterEach(cleanup);

const SCOPE: WidgetInfoScope = { instance: "jobs", reference: "omfp:jobs", workspaceId: "w1" };

let latest: Jobs | undefined;

function Probe({ workspaceId }: { workspaceId?: string }) {
  latest = useJobs(workspaceId);
  return (
    <span data-testid="jobs">{`${latest.loaded} ${latest.jobs.map((job) => job.name).join(",")} ${latest.error}`}</span>
  );
}

function jobs(): Jobs {
  if (latest === undefined) {
    throw new Error("not rendered");
  }
  return latest;
}

describe("useJobs", () => {
  // spec: AGT-090
  test("follows the widget's workspace's jobs: [] and not loaded until the state arrives, then live", async () => {
    const test = renderWithHost(
      <WidgetScope value={SCOPE}>
        <Probe />
      </WidgetScope>,
    );
    test.flush();
    expect(test.client.subscribed).toEqual([jobsTopic("w1")]);
    expect(screen.getByTestId("jobs").textContent).toBe("false  null");

    test.client.jobs("w1", { jobs: [jobView("build"), jobView("deploy")] });
    test.flush();
    expect(screen.getByTestId("jobs").textContent).toBe("true build,deploy null");
    test.client.jobsEvents("w1", { kind: "jobs_set", jobs: [jobView("build")], error: "jobs.x: unknown key" });
    test.flush();
    expect(screen.getByTestId("jobs").textContent).toBe("true build jobs.x: unknown key");

    test.view.unmount();
    await released();
    expect(test.client.unsubscribed).toEqual([jobsTopic("w1")]);
  });

  // spec: AGT-090
  test("run, kill and enable call the methods by name and reject with their errors", async () => {
    const test = renderWithHost(<Probe workspaceId="w2" />);
    test.client.jobs("w2", {
      jobs: [jobView("build"), jobView("watch", {}, { status: "running", run: runInfo("r7", "watch") })],
    });
    test.flush();
    expect(test.client.subscribed).toEqual([jobsTopic("w2")]);

    const run = jobs().run("build");
    test.client.callsTo("jobs.run")[0]?.resolve(runInfo("r8", "build"));
    expect((await run).id).toBe("r8");
    const enable = jobs().enable("build", false);
    test.client.callsTo("jobs.enable")[0]?.resolve(jobView("build", { enabled: false }, { status: "disabled" }));
    expect((await enable).status).toBe("disabled");
    const kill = jobs().kill("watch");
    test.client.callsTo("jobs.kill")[0]?.resolve({});
    await kill;
    expect(test.client.calls.map((call) => [call.method, call.params])).toEqual([
      ["jobs.run", { workspaceId: "w2", job: "build" }],
      ["jobs.enable", { workspaceId: "w2", name: "build", enabled: false }],
      ["jobs.kill", { jobId: "r7" }],
    ]);

    await expect(jobs().kill("build")).rejects.toThrow("build is not running");
    await expect(jobs().kill("nope")).rejects.toThrow("No job nope in this workspace");
    const refused = jobs().run("watch");
    const conflict = new CallError({ kind: "daemon", error: { code: "conflict", message: "watch runs" } });
    act(() => test.client.callsTo("jobs.run")[1]?.reject(conflict));
    await expect(refused).rejects.toBe(conflict);
  });

  // spec: AGT-090
  test("without a workspace nothing is followed and every action rejects", async () => {
    renderWithHost(<Probe />);
    expect(jobs().loaded).toBe(false);
    await expect(jobs().run("x")).rejects.toThrow("No workspace");
  });
});
