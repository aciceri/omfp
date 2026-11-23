/**
 * `just check-workflow`: actionlint, then the GitHub workflows against the justfile (`check`, tested by
 * `check-workflow.test.ts`). Run from the repository root inside the dev shell: `bun .github/check-workflow.ts <just
 * executable>`.
 *
 * - TST-070: CI runs on pushes to master and on pull requests; each recipe job sets up the dev shell and runs only `nix
 *   develop -c just <recipe>`; every recipe `just ci` runs has a job that runs it on pull requests too (no `if:`), and
 *   no job runs a recipe `just ci` does not.
 * - TST-071: actionlint passes; every action of every workflow (ENV-051 too) is pinned by a major tag or a commit SHA.
 *   The other rules are about `ci.yml` only.
 * - TST-072: the Nix store comes from the `omfp` Cachix cache, which pushes to master fill, and on pull requests from
 *   this repository the `dev-shell` job alone (`push-pull-request`); the cargo cache is saved on master only, by the
 *   jobs with a target cache; a job without one (the mutation gate's) restores the cache of a job that saves one; the
 *   git checkouts are one cache of their own, keyed on Cargo.lock's git sources, filled and saved on master by the
 *   `dev-shell` job alone, restored by every Rust job and linked into CARGO_HOME after rust-cache; the dev-shell
 *   action reports the cargo cache in the job summary; every recipe job hands the dev-shell action the Cachix token.
 * - TST-073: every job running the e2e or desktop tests builds its own web bundle: it sets no `OMFP_WEB_BUNDLE`,
 *   downloads no artifact and waits for no job but `dev-shell`.
 * - TST-075: before installing Nix, the dev-shell action pools the runner's disk into a Btrfs /nix with
 *   `wimpysworld/nothing-but-nix` (Linux runners), Nix builds in /nix/build, and the cachix daemon pushes every path as
 *   it is built.
 * - TST-076: the `dev-shell` job runs no recipe and every other Linux recipe job needs it; CARGO_HOME lives on the Nix volume.
 * - TST-118: one aggregate `required CI` job waits for every blocking job even after failures or skips, while the
 *   full mutation gate remains visible but non-blocking.
 * - TST-092: one job on a macOS runner runs `just test-desktop` and `just package` on pushes to master, by hand and on
 *   pull requests labeled `ci/macos` only; `ci.yml` runs by hand, and `ci-macos-label.yml` runs it on the branch of a
 *   pull request that gets that label (and only that one). In a run by hand that job reports itself as the commit
 *   status `macOS (ci/macos)` (pending first, its result last), with `statuses: write` on that job only.
 * - ENV-060: `release.yml` runs on `v*` tags and by hand; its jobs build the packages in the dev shell (`just
 *   package-linux` and `just package-flatpak` on Linux, `just package-macos` on macOS); only its job that runs on tags
 *   alone may write the repository's contents, the workflow reads them.
 */
import { $ } from "bun";

export interface Step {
  id?: string;
  uses?: string;
  run?: string;
  if?: string;
  env?: Record<string, string>;
  with?: Record<string, string>;
}
export interface Job {
  name?: string;
  "runs-on"?: string;
  if?: string;
  needs?: string | string[];
  env?: Record<string, string>;
  permissions?: Record<string, string>;
  steps?: Step[];
}
export interface Workflow {
  on?: {
    push?: { branches?: string[]; tags?: string[] };
    pull_request?: { types?: string[] } | null;
    workflow_dispatch?: unknown;
  };
  permissions?: Record<string, string>;
  jobs: Record<string, Job>;
}
export interface Action {
  runs: { steps: Step[] };
}
/** `just --dump --dump-format json`, the part read here: the recipes' names. */
export interface Recipes {
  recipes: Record<string, unknown>;
}
/**
 * What the check reads: `ci.yml`, the dev-shell action, the justfile's recipes and the ones `just ci` runs (its
 * `ci_recipes`), `release.yml`, `ci-macos-label.yml` and every workflow.
 */
export interface Inputs {
  workflow: Workflow;
  setup: Action;
  recipes: Recipes["recipes"];
  ci: string[];
  release: Workflow;
  macosLabel: Workflow;
  workflows: Workflow[];
}

export const WORKFLOW = ".github/workflows/ci.yml";
export const SETUP = ".github/actions/dev-shell/action.yml";
export const RELEASE = ".github/workflows/release.yml";
export const MACOS_LABEL = ".github/workflows/ci-macos-label.yml";
const SETUP_USES = "./.github/actions/dev-shell";
const RUN = /^nix develop -c just ([\w-]+)$/;
/** The GitHub expression `${{ inner }}`. */
const expression = (inner: string) => `\${{ ${inner} }}`;
const ON_MASTER = "github.ref == 'refs/heads/master'";
const SHELL_JOB = "dev-shell";
/** The shared cache of the cargo git checkouts (TST-072), the same path on every runner. */
const CARGO_GIT = "/var/tmp/cargo-git";
const REQUIRED_GATE = "required";
const REQUIRED_JOBS = ["check", "test-ts", "test-contract", "test-e2e", "test-desktop", "package", "cov"];
const REQUIRED_RUN = `jq -e 'all(.[]; .result == "success")' <<<"$RESULTS"`;

/** TST-092: the step of the macOS job reporting a run by hand as a commit status of its commit, the state in `STATE`. */
const MACOS_STATUS_RUN =
  'gh api "repos/$GITHUB_REPOSITORY/statuses/$GITHUB_SHA" -f state="$STATE" -f context="macOS (ci/macos)" -f target_url="$GITHUB_SERVER_URL/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID"';
const BY_HAND = "github.event_name == 'workflow_dispatch'";
const isMacosStatus = (step: Step) => step.run?.trim() === MACOS_STATUS_RUN;

/** The recipe a step runs (`nix develop -c just <recipe>`), if it is one. */
const recipeOf = (step: Step) => RUN.exec(step.run?.trim() ?? "")?.[1];

/** The jobs running each recipe, in workflow order. */
function jobsByRecipe(workflow: Workflow): Map<string, string[]> {
  const ranBy = new Map<string, string[]>();
  for (const [id, job] of Object.entries(workflow.jobs)) {
    for (const step of job.steps ?? []) {
      const recipe = recipeOf(step);
      if (recipe !== undefined) ranBy.set(recipe, [...(ranBy.get(recipe) ?? []), id]);
    }
  }
  return ranBy;
}

/** TST-070 */
function checkRecipes({ workflow, recipes, ci }: Inputs, ranBy: Map<string, string[]>, problems: string[]) {
  if (!workflow.on?.push?.branches?.includes("master") || !workflow.on || !("pull_request" in workflow.on)) {
    problems.push(`${WORKFLOW} must run on pushes to master and on pull requests`);
  }
  for (const [id, job] of Object.entries(workflow.jobs)) {
    if (id === REQUIRED_GATE) continue;
    const steps = job.steps ?? [];
    if (!steps.some((step) => step.uses === SETUP_USES)) {
      problems.push(`job ${id} does not use ${SETUP_USES}`);
    }
    for (const step of steps) {
      if (step.run === undefined || isMacosStatus(step)) continue;
      const recipe = recipeOf(step);
      if (recipe === undefined) {
        problems.push(`job ${id} runs \`${step.run.trim()}\`, not \`nix develop -c just <recipe>\``);
      } else if (!(recipe in recipes)) {
        problems.push(`job ${id} runs \`just ${recipe}\`, which the justfile does not define`);
      }
    }
  }
  for (const recipe of ci) {
    const always = (ranBy.get(recipe) ?? []).filter((id) => workflow.jobs[id]?.if === undefined);
    if (always.length === 0) {
      problems.push(`\`just ci\` runs ${recipe}, no job of ${WORKFLOW} does on every run (pull requests too)`);
    }
  }
  for (const [recipe, ids] of ranBy) {
    if (ci.includes(recipe) || !(recipe in recipes)) continue;
    for (const id of ids) problems.push(`job ${id} runs \`just ${recipe}\`, which \`just ci\` does not`);
  }
}

/** TST-118: the one status branch protection requires; mutation testing deliberately stays outside it. */
function checkRequiredGate({ workflow }: Inputs, problems: string[]) {
  const gate = workflow.jobs[REQUIRED_GATE];
  if (gate === undefined) {
    problems.push("job required must aggregate the blocking CI jobs");
    return;
  }
  if (gate.name !== "required CI") {
    problems.push("job required must be named `required CI`");
  }
  const needs = [gate.needs ?? []].flat();
  if (needs.length !== REQUIRED_JOBS.length || needs.some((id, index) => id !== REQUIRED_JOBS[index])) {
    problems.push(`job required must need exactly ${REQUIRED_JOBS.join(", ")}`);
  }
  if (gate.if !== "always()") {
    problems.push("job required must run with `always()`");
  }
  const steps = gate.steps ?? [];
  if (
    gate["runs-on"] !== "ubuntu-latest" ||
    steps.length !== 1 ||
    steps[0]?.env?.RESULTS !== expression("toJSON(needs)") ||
    steps[0]?.run?.trim() !== REQUIRED_RUN
  ) {
    problems.push("job required must fail unless every blocking job succeeded");
  }
}

/** TST-071 */
function checkPins({ setup, workflows }: Inputs, problems: string[]) {
  const PINNED = /^[^@]+@(v\d+|[0-9a-f]{40})$/;
  const steps = [
    ...workflows.flatMap((other) => Object.values(other.jobs).flatMap((job) => job.steps ?? [])),
    ...setup.runs.steps,
  ];
  for (const { uses } of steps) {
    if (uses !== undefined && !uses.startsWith("./") && !PINNED.test(uses)) {
      problems.push(`${uses} is pinned neither by a major tag nor by a commit SHA`);
    }
  }
}

const setupStep = (setup: Action, action: string) =>
  setup.runs.steps.find((step) => step.uses?.startsWith(`${action}@`));

/** TST-072 */
function checkCaches(inputs: Inputs, ranBy: Map<string, string[]>, problems: string[]) {
  const { workflow, setup } = inputs;
  const cachix = setupStep(setup, "cachix/cachix-action");
  if (cachix?.with?.name !== "omfp") problems.push(`${SETUP} must use the omfp Cachix cache`);
  const pushes = [
    `github.event_name == 'push' && ${ON_MASTER}`,
    "inputs.push-pull-request == 'true' && github.event.pull_request.head.repo.full_name == github.repository",
  ];
  if (cachix?.with?.skipPush !== expression(`!(${pushes.join(" || ")})`)) {
    problems.push(`${SETUP} must push to Cachix on pushes to master, and on pull requests from this repository`);
  }
  for (const [id, job] of Object.entries(workflow.jobs)) {
    const setupWith = job.steps?.find((step) => step.uses === SETUP_USES)?.with;
    if (id !== SHELL_JOB && setupWith?.["push-pull-request"] !== undefined) {
      problems.push(`job ${id} must not push to Cachix on pull requests: only ${SHELL_JOB} does`);
    }
  }
  const rustCache = setupStep(setup, "Swatinem/rust-cache");
  if (rustCache?.with?.["save-if"] !== expression(`inputs.target-cache == 'true' && ${ON_MASTER}`)) {
    problems.push(`${SETUP} must save the cargo cache on master only, and only for jobs with a target cache`);
  }
  // A job without a target cache restores a saving job's entry: the same key and paths (target/ included).
  const lender = /^\$\{\{ inputs\.target-cache == 'false' && '([\w-]+)' \|\| '' \}\}$/.exec(
    rustCache?.with?.["shared-key"] ?? "",
  )?.[1];
  const lenderSetup = workflow.jobs[lender ?? ""]?.steps?.find((step) => step.uses === SETUP_USES)?.with;
  if (
    lender === undefined ||
    workflow.jobs[lender]?.if !== undefined ||
    lenderSetup === undefined ||
    lenderSetup.rust === "false" ||
    lenderSetup["target-cache"] === "false" ||
    rustCache?.with?.["cache-targets"] !== undefined
  ) {
    problems.push(`${SETUP} must restore a job without a target cache from the cargo cache of a job that saves one`);
  }
  const report = setupStep(setup, "srz-zumix/post-run-action")?.with?.["post-run"] ?? "";
  if (!report.includes("steps.rust-cache.outputs.cache-hit") || !report.includes("$GITHUB_STEP_SUMMARY")) {
    problems.push(`${SETUP} must report the cargo cache's hit and sizes in the job summary`);
  }
  checkCargoGit(inputs, problems);
  for (const [id, job] of Object.entries(workflow.jobs)) {
    if (id === REQUIRED_GATE) continue;
    const token = job.steps?.find((step) => step.uses === SETUP_USES)?.with?.["cachix-auth-token"];
    if (token !== expression("secrets.CACHIX_AUTH_TOKEN")) {
      problems.push(`job ${id} must hand ${SETUP_USES} the CACHIX_AUTH_TOKEN secret`);
    }
  }
  for (const id of ranBy.get("mutants-gate") ?? []) {
    const setupWith = workflow.jobs[id]?.steps?.find((step) => step.uses === SETUP_USES)?.with;
    if (setupWith?.["target-cache"] !== "false") {
      problems.push(
        `job ${id} must not keep a cargo cache of its own (target-cache: "false"): cargo mutants builds in copies`,
      );
    }
  }
}

/** TST-072: the git checkouts are one cache, which the per-job cargo caches leave out. */
function checkCargoGit({ workflow, setup }: Inputs, problems: string[]) {
  const steps = setup.runs.steps;
  const key = expression("steps.cargo-git.outputs.key");
  const keyStep = steps.find((step) => step.id === "cargo-git");
  if (!/grep '\^source = "git\+' Cargo\.lock/.test(keyStep?.run ?? "")) {
    problems.push(`${SETUP} must key the cargo git cache on Cargo.lock's git sources`);
  }
  const restore = steps.find((step) => step.uses?.startsWith("actions/cache/restore@"));
  if (
    restore?.id !== "cargo-git-cache" ||
    restore.with?.path !== CARGO_GIT ||
    restore.with?.key !== key ||
    restore.if !== "inputs.rust == 'true' || inputs.fill-cargo-git == 'true'"
  ) {
    problems.push(`${SETUP} must restore the cargo git cache in every Rust job`);
  }
  const saves = steps.filter(
    (step) => step.uses?.startsWith("actions/cache@") || step.uses?.startsWith("actions/cache/save@"),
  );
  const fillIf = `inputs.fill-cargo-git == 'true' && steps.cargo-git-cache.outputs.cache-hit != 'true' && ${ON_MASTER}`;
  if (
    saves.length !== 1 ||
    saves[0]?.uses?.startsWith("actions/cache/save@") !== true ||
    saves[0].with?.path !== CARGO_GIT ||
    saves[0].with?.key !== key ||
    saves[0].if !== fillIf
  ) {
    problems.push(`${SETUP} must save the cargo git cache on master only, from the job that fills it`);
  }
  const rustCache = steps.findIndex((step) => step.uses?.startsWith("Swatinem/rust-cache@"));
  const linked = steps.some(
    (step, index) =>
      index > rustCache &&
      step.if === "inputs.rust == 'true'" &&
      /^git_dir="\$\{CARGO_HOME:-\$HOME\/\.cargo\}\/git"$/m.test(step.run ?? "") &&
      (step.run ?? "").includes(`ln -sfn ${CARGO_GIT} "$git_dir"`),
  );
  if (rustCache === -1 || !linked) {
    problems.push(
      `${SETUP} must link CARGO_HOME/git to the cargo git cache after rust-cache, which then leaves it out`,
    );
  }
  const fillers = Object.entries(workflow.jobs)
    .filter(([, job]) =>
      job.steps?.some((step) => step.uses === SETUP_USES && step.with?.["fill-cargo-git"] === "true"),
    )
    .map(([id]) => id);
  if (fillers.length !== 1 || fillers[0] !== SHELL_JOB) {
    problems.push(`job ${SHELL_JOB}, and no other, must fill the cargo git cache (fill-cargo-git: "true")`);
  }
}

/** TST-073: every job running the e2e or desktop tests builds its own web bundle (`with-web-bundle`, seconds). */
function checkBundle({ workflow }: Inputs, ranBy: Map<string, string[]>, problems: string[]) {
  const ids = new Set(["test-e2e", "test-desktop"].flatMap((recipe) => ranBy.get(recipe) ?? []));
  for (const id of ids) {
    const job = workflow.jobs[id];
    const waits = [job?.needs ?? []].flat().filter((need) => need !== SHELL_JOB);
    const download = job?.steps?.some((step) => step.uses?.startsWith("actions/download-artifact@"));
    if (job?.env?.OMFP_WEB_BUNDLE !== undefined || waits.length > 0 || download) {
      problems.push(`job ${id} must build its own web bundle, waiting for no job but ${SHELL_JOB}`);
    }
  }
}

/** TST-075 */
function checkDisk({ setup }: Inputs, problems: string[]) {
  const setupIndex = (action: string) => setup.runs.steps.findIndex((step) => step.uses?.startsWith(`${action}@`));
  const pool = setupIndex("wimpysworld/nothing-but-nix");
  const install = setupIndex("cachix/install-nix-action");
  if (pool === -1 || install === -1 || pool > install) {
    problems.push(`${SETUP} must run wimpysworld/nothing-but-nix before cachix/install-nix-action`);
  }
  if (!/^build-dir = \/nix\/build$/m.test(setup.runs.steps[install]?.with?.extra_nix_config ?? "")) {
    problems.push(`${SETUP} must make Nix build in /nix/build (extra_nix_config: build-dir = /nix/build)`);
  }
  if (setupStep(setup, "cachix/cachix-action")?.with?.useDaemon === "false") {
    problems.push(`${SETUP} must keep the cachix daemon, which pushes what a failing job built`);
  }
}

/** TST-076 */
function checkShellJob({ workflow, setup }: Inputs, problems: string[]) {
  const isLinux = (job: Job) => job["runs-on"]?.startsWith("ubuntu-") ?? false;
  const shellJob = workflow.jobs[SHELL_JOB];
  if (shellJob === undefined || !isLinux(shellJob) || (shellJob.steps ?? []).some((step) => step.run !== undefined)) {
    problems.push(`job ${SHELL_JOB} must only set up the dev shell on a Linux runner`);
  }
  for (const [id, job] of Object.entries(workflow.jobs)) {
    if (
      id !== SHELL_JOB &&
      id !== REQUIRED_GATE &&
      isLinux(job) &&
      ![job.needs ?? []].flat().includes(SHELL_JOB)
    ) {
      problems.push(`job ${id} must need ${SHELL_JOB}, which builds the Linux dev shell once`);
    }
  }
  const cargoHome = setup.runs.steps.find((step) => /CARGO_HOME=/.test(step.run ?? ""));
  if (!/^\s*echo "CARGO_HOME=\/var\/tmp\/nothing-but-nix\/[^"]+" >> "\$GITHUB_ENV"$/m.test(cargoHome?.run ?? "")) {
    problems.push(`${SETUP} must put CARGO_HOME on the Nix volume (/var/tmp/nothing-but-nix)`);
  }
}

/** TST-092 */
function checkMacos({ workflow, macosLabel }: Inputs, problems: string[]) {
  const MACOS_RECIPES = ["test-desktop", "package"];
  const MACOS_IF =
    "github.event_name != 'pull_request' || contains(github.event.pull_request.labels.*.name, 'ci/macos')";
  const LABEL_IF =
    "github.event.label.name == 'ci/macos' && github.event.pull_request.head.repo.full_name == github.repository";
  const DISPATCH = 'gh workflow run ci.yml --repo "$GITHUB_REPOSITORY" --ref "$BRANCH"';
  const macos = Object.values(workflow.jobs).filter((job) => /^macos-\d+$/.test(job["runs-on"] ?? ""));
  const labeled = macos.some((job) => {
    const ran = (job.steps ?? []).map(recipeOf);
    return job.if === MACOS_IF && MACOS_RECIPES.every((recipe) => ran.includes(recipe));
  });
  if (!labeled) {
    problems.push(
      `one job on a macOS runner must run ${MACOS_RECIPES.join(" and ")} on pushes to master, by hand and on pull requests labeled ci/macos only`,
    );
  }
  if (!workflow.on || !("workflow_dispatch" in workflow.on)) {
    problems.push(`${WORKFLOW} must run by hand (workflow_dispatch) for the ci/macos label`);
  }
  // The pull request's own run shows the job skipped; the run by hand the label starts is no check of the pull
  // request, so the job reports that run on its commit, the pull request's head.
  const status = (step: Step | undefined, condition: string, state: string) =>
    step !== undefined &&
    isMacosStatus(step) &&
    step.if === condition &&
    step.env?.STATE === state &&
    step.env.GH_TOKEN === expression("github.token");
  const RESULT = expression("job.status == 'success' && 'success' || job.status == 'cancelled' && 'error' || 'failure'");
  const reports = macos.some((job) => {
    const steps = job.steps ?? [];
    return (
      job.if === MACOS_IF &&
      job.permissions?.statuses === "write" &&
      status(steps[0], BY_HAND, "pending") &&
      status(steps.at(-1), `always() && ${BY_HAND}`, RESULT)
    );
  });
  const writers = Object.values(workflow.jobs).filter((job) => job.permissions?.statuses === "write");
  if (!reports || writers.length !== 1 || workflow.permissions?.statuses !== undefined) {
    problems.push(
      'the macOS job must report a run by hand as the commit status "macOS (ci/macos)" of its commit, pending first and its result last, with statuses: write on that job only',
    );
  }
  const triggers = Object.keys(macosLabel.on ?? {});
  const types = macosLabel.on?.pull_request?.types ?? [];
  const dispatches = Object.values(macosLabel.jobs).some(
    (job) =>
      job.if === LABEL_IF &&
      job.permissions?.actions === "write" &&
      (job.steps ?? []).some(
        (step) =>
          step.run?.trim() === DISPATCH &&
          step.env?.BRANCH === expression("github.event.pull_request.head.ref") &&
          step.env.GH_TOKEN === expression("github.token"),
      ),
  );
  if (triggers.join() !== "pull_request" || types.join() !== "labeled" || !dispatches) {
    problems.push(
      `${MACOS_LABEL} must run ${WORKFLOW} on the branch of a pull request labeled ci/macos, and only then`,
    );
  }
}

/** ENV-060 */
function checkRelease({ release }: Inputs, problems: string[]) {
  const RELEASE_RECIPES: Record<string, RegExp> = {
    "package-linux": /^ubuntu-/,
    "package-flatpak": /^ubuntu-/,
    "package-macos": /^macos-\d+$/,
  };
  if (!release.on?.push?.tags?.includes("v*") || !release.on || !("workflow_dispatch" in release.on)) {
    problems.push(`${RELEASE} must run on pushed v* tags and by hand (workflow_dispatch)`);
  }
  if (release.permissions?.contents !== "read") problems.push(`${RELEASE} must only read the contents by default`);
  for (const [recipe, runner] of Object.entries(RELEASE_RECIPES)) {
    const builds = Object.values(release.jobs).some(
      (job) =>
        runner.test(job["runs-on"] ?? "") &&
        (job.steps ?? []).some((step) => step.uses === SETUP_USES) &&
        (job.steps ?? []).some((step) => recipeOf(step) === recipe),
    );
    if (!builds) problems.push(`${RELEASE}: a job on a ${runner} runner must run \`just ${recipe}\` in the dev shell`);
  }
  for (const [id, job] of Object.entries(release.jobs)) {
    if (job.permissions?.contents === "write" && job.if !== "github.ref_type == 'tag'") {
      problems.push(`${RELEASE}: job ${id} writes the contents, so it must run on tags only`);
    }
  }
}

/** Every problem of the workflows against the justfile, once each. */
export function check(inputs: Inputs): string[] {
  const problems: string[] = [];
  const ranBy = jobsByRecipe(inputs.workflow);
  checkRecipes(inputs, ranBy, problems);
  checkRequiredGate(inputs, problems);
  checkPins(inputs, problems);
  checkCaches(inputs, ranBy, problems);
  checkBundle(inputs, ranBy, problems);
  checkDisk(inputs, problems);
  checkShellJob(inputs, problems);
  checkMacos(inputs, problems);
  checkRelease(inputs, problems);
  return [...new Set(problems)];
}

const yaml = async <T>(path: string) => Bun.YAML.parse(await Bun.file(path).text()) as T;

/** The repository's workflows and the recipes of `just` (run from the repository root). */
export async function read(just: string): Promise<Inputs> {
  const paths = [...new Bun.Glob(".github/workflows/*.{yml,yaml}").scanSync()];
  return {
    workflow: await yaml<Workflow>(WORKFLOW),
    setup: await yaml<Action>(SETUP),
    recipes: ((await $`${just} --dump --dump-format json`.json()) as Recipes).recipes,
    ci: (await $`${just} --evaluate ci_recipes`.text()).trim().split(/\s+/),
    release: await yaml<Workflow>(RELEASE),
    macosLabel: await yaml<Workflow>(MACOS_LABEL),
    workflows: await Promise.all(paths.map((path) => yaml<Workflow>(path))),
  };
}

if (import.meta.main) {
  await $`actionlint`;
  const problems = check(await read(process.argv[2] ?? "just"));
  if (problems.length > 0) {
    console.error(problems.join("\n"));
    process.exit(1);
  }
}
