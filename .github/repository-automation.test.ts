// Run by `just check-workflow` (`bun test ./.github`): the flake's binary cache and the automated update pull requests.
import { expect, test } from "bun:test";
import type { Workflow } from "./check-workflow.ts";

/** A file of the repository, by its path from the root. */
const text = (path: string) => Bun.file(new URL(`../${path}`, import.meta.url)).text();

// spec: ENV-034
test("the flake's nixConfig names the project's binary cache", async () => {
  const nixConfig = /^ {2}nixConfig = \{\n([\s\S]*?)\n {2}\};$/m.exec(await text("flake.nix"))?.[1];
  expect(nixConfig).toContain('extra-substituters = [ "https://omfp.cachix.org" ];');
  expect(nixConfig).toMatch(
    /extra-trusted-public-keys = \[\s*"omfp\.cachix\.org-1:WcxTiZMXSYmi9k6fxowUuIMmszu9aWymk2djl9kXjHc="\s*\];/,
  );
});

// spec: ENV-050
test("one weekly Dependabot pull request updates the actions of the workflows and the dev-shell action", async () => {
  expect(Bun.YAML.parse(await text(".github/dependabot.yml"))).toEqual({
    version: 2,
    updates: [
      {
        "package-ecosystem": "github-actions",
        directories: ["/", "/.github/actions/dev-shell"],
        schedule: { interval: "weekly" },
        "commit-message": { prefix: "ci", include: "scope" },
        groups: { "github-actions": { patterns: ["*"] } },
      },
    ],
  });
});

// spec: ENV-051
test("a weekly pull request updates every flake input but oh-my-pi", async () => {
  const workflow = Bun.YAML.parse(await text(".github/workflows/update-flake-lock.yml")) as Workflow & {
    on: { schedule?: { cron: string }[] };
  };
  expect(workflow.on.schedule).toEqual([{ cron: "0 4 * * 1" }]);
  expect(workflow.on).toHaveProperty("workflow_dispatch");
  expect(workflow.permissions).toEqual({ contents: "read" });
  const jobs = Object.values(workflow.jobs);
  expect(jobs).toHaveLength(1);
  expect(jobs[0]?.permissions).toEqual({ contents: "write", "pull-requests": "write" });
  const update = jobs[0]?.steps?.find((step) => step.uses?.startsWith("DeterminateSystems/update-flake-lock@"));
  expect(update?.with?.["commit-msg"]).toBe("chore(nix): update flake.lock");
  expect(update?.with?.["pr-title"]).toBe("chore(nix): update flake.lock");

  const lock = JSON.parse(await text("flake.lock")) as { nodes: { root: { inputs: Record<string, string> } } };
  const inputs = Object.keys(lock.nodes.root.inputs).filter((input) => input !== "oh-my-pi");
  expect(update?.with?.inputs?.split(/\s+/).sort()).toEqual(inputs.sort());
});
