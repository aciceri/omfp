// Run by `just check-workflow` from the repository root (`bun test ./.github`).
import { describe, expect, test } from "bun:test";
import { type Action, check, type Inputs, MACOS_LABEL, read, WORKFLOW, type Workflow } from "./check-workflow.ts";

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

/** The repository's inputs with the dev-shell action changed by `edit` (on a copy). */
function withSetup(edit: (setup: Action) => void): Inputs {
  const setup = structuredClone(repository.setup);
  edit(setup);
  return { ...repository, setup };
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

  // spec: TST-072
  test("a mutation job keeping a cargo cache of its own fails", () => {
    const problems = check(
      withWorkflow((workflow) => {
        const setup = workflow.jobs.mutants?.steps?.find((step) => step.uses === "./.github/actions/dev-shell");
        if (setup?.with !== undefined) delete setup.with["target-cache"];
      }),
    );
    expect(problems).toContain(
      'job mutants must not keep a cargo cache of its own (target-cache: "false"): cargo mutants builds in copies',
    );
  });

  // spec: TST-072
  test("a dev-shell action without the cargo cache report fails", () => {
    const problems = check(
      withSetup((setup) => {
        setup.runs.steps = setup.runs.steps.filter((step) => !step.uses?.startsWith("srz-zumix/post-run-action@"));
      }),
    );
    expect(problems).toContain(
      ".github/actions/dev-shell/action.yml must report the cargo cache's hit and sizes in the job summary",
    );
  });

  // spec: TST-072
  test("a job without a target cache borrowing from a job that saves none fails", () => {
    const borrowFrom = (lender: string) =>
      check(
        withSetup((setup) => {
          const rustCache = setup.runs.steps.find((step) => step.uses?.startsWith("Swatinem/rust-cache@"));
          if (rustCache?.with !== undefined) {
            rustCache.with["shared-key"] = `\${{ inputs.target-cache == 'false' && '${lender}' || '' }}`;
          }
        }),
      );
    const problem =
      ".github/actions/dev-shell/action.yml must restore a job without a target cache from the cargo cache of a job that saves one";
    expect(borrowFrom("mutants")).toContain(problem);
    expect(borrowFrom("package")).toContain(problem);
    expect(borrowFrom("macos")).toContain(problem);
    expect(borrowFrom("cov")).not.toContain(problem);
  });

  // spec: TST-072
  test("saving the cargo cache of a job without a target cache fails", () => {
    const problems = check(
      withSetup((setup) => {
        const rustCache = setup.runs.steps.find((step) => step.uses?.startsWith("Swatinem/rust-cache@"));
        if (rustCache?.with !== undefined) rustCache.with["save-if"] = "${{ github.ref == 'refs/heads/master' }}";
      }),
    );
    expect(problems).toContain(
      ".github/actions/dev-shell/action.yml must save the cargo cache on master only, and only for jobs with a target cache",
    );
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
