// Run by `just check-workflow` from the repository root (`bun test ./.github`).
import { describe, expect, test } from "bun:test";
import { check, type Inputs, MACOS_LABEL, read, WORKFLOW, type Workflow } from "./check-workflow.ts";

const repository = await read(process.env.JUST ?? "just");

/** The repository's inputs with `ci.yml` changed by `edit` (on a copy). */
function withWorkflow(edit: (workflow: Workflow) => void): Inputs {
  const workflow = structuredClone(repository.workflow);
  edit(workflow);
  return { ...repository, workflow };
}

/** The repository's inputs with `ci-macos-label.yml` changed by `edit` (on a copy). */
function withMacosLabel(edit: (workflow: Workflow) => void): Inputs {
  const macosLabel = structuredClone(repository.macosLabel);
  edit(macosLabel);
  return { ...repository, macosLabel };
}

describe("check-workflow", () => {
  // spec: TST-112
  test("the repository's workflows pass", () => {
    expect(check(repository)).toEqual([]);
  });

  // spec: TST-112
  test("removing the Linux desktop job fails, though the macOS job runs the recipe on pushes", () => {
    const problems = check(
      withWorkflow((workflow) => {
        delete workflow.jobs["test-desktop"];
      }),
    );
    expect(problems).toContain(
      "`just ci` runs test-desktop, no job of .github/workflows/ci.yml does on every run (pull requests too)",
    );
  });

  // spec: TST-112
  test("removing the Linux package job fails", () => {
    const problems = check(
      withWorkflow((workflow) => {
        delete workflow.jobs.package;
      }),
    );
    expect(problems).toContain(
      "`just ci` runs package, no job of .github/workflows/ci.yml does on every run (pull requests too)",
    );
  });

  // spec: TST-112
  test("a job running a recipe `just ci` does not fails", () => {
    const problems = check(
      withWorkflow((workflow) => {
        workflow.jobs.check?.steps?.push({ run: "nix develop -c just serve" });
      }),
    );
    expect(problems).toContain("job check runs `just serve`, which `just ci` does not");
  });

  // spec: TST-112
  test("every job running the desktop tests serves the shared bundle, the Linux one too", () => {
    const problems = check(
      withWorkflow((workflow) => {
        const job = workflow.jobs["test-desktop"];
        if (job !== undefined) delete job.env;
      }),
    );
    expect(problems).toContain("job test-desktop must serve the bundle of job web through OMFP_WEB_BUNDLE");
  });

  // spec: TST-118
  test("removing the required aggregate gate fails", () => {
    const problems = check(
      withWorkflow((workflow) => {
        delete workflow.jobs.required;
      }),
    );
    expect(problems).toContain("job required must aggregate the blocking CI jobs");
  });

  // spec: TST-118
  test("the required gate waits for every blocking job except mutation testing", () => {
    const problems = check(
      withWorkflow((workflow) => {
        const required = workflow.jobs.required;
        if (required !== undefined) required.needs = ["check", "mutants"];
      }),
    );
    expect(problems).toContain(
      "job required must need exactly check, test-ts, test-contract, test-e2e, test-desktop, package, cov",
    );
  });

  // spec: TST-118
  test("the required gate runs after failed or skipped dependencies", () => {
    const problems = check(
      withWorkflow((workflow) => {
        const required = workflow.jobs.required;
        if (required !== undefined) delete required.if;
      }),
    );
    expect(problems).toContain("job required must run with `always()`");
  });

  // spec: TST-118
  test("the required gate fails unless every dependency succeeded", () => {
    const problems = check(
      withWorkflow((workflow) => {
        const required = workflow.jobs.required;
        const step = required?.steps?.[0];
        if (step !== undefined) step.run = "true";
      }),
    );
    expect(problems).toContain("job required must fail unless every blocking job succeeded");
  });

  // spec: TST-118
  test("the required gate keeps the status name configured in the ruleset", () => {
    const problems = check(
      withWorkflow((workflow) => {
        const required = workflow.jobs.required as (typeof workflow.jobs)[string] & { name?: string };
        required.name = "renamed";
      }),
    );
    expect(problems).toContain("job required must be named `required CI`");
  });

  // spec: TST-092
  test("the macOS job runs on pull requests only with the ci/macos label", () => {
    const problems = check(
      withWorkflow((workflow) => {
        const macos = workflow.jobs.macos;
        if (macos !== undefined) delete macos.if;
      }),
    );
    expect(problems).toContain(
      "one job on a macOS runner must run test-desktop and package on pushes to master, by hand and on pull requests labeled ci/macos only",
    );
  });

  // spec: TST-092
  test("CI runs by hand, as the ci/macos label starts it", () => {
    const problems = check(
      withWorkflow((workflow) => {
        delete workflow.on?.workflow_dispatch;
      }),
    );
    expect(problems).toContain(`${WORKFLOW} must run by hand (workflow_dispatch) for the ci/macos label`);
  });

  // spec: TST-092
  test("only the ci/macos label starts CI on a pull request's branch", () => {
    const message = `${MACOS_LABEL} must run ${WORKFLOW} on the branch of a pull request labeled ci/macos, and only then`;
    const anyLabel = check(
      withMacosLabel((workflow) => {
        for (const job of Object.values(workflow.jobs)) delete job.if;
      }),
    );
    expect(anyLabel).toContain(message);
    const everyPush = check(
      withMacosLabel((workflow) => {
        workflow.on = { pull_request: null };
      }),
    );
    expect(everyPush).toContain(message);
  });
});
