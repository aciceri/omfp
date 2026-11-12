/**
 * `just check-workflow` (TST-070..TST-073): actionlint, then the GitHub workflow against the justfile. Run from the
 * repository root inside the dev shell: `bun .github/check-workflow.ts <just executable>`.
 *
 * - TST-070: CI runs on pushes to master and on pull requests; each job sets up the dev shell and runs only `nix develop
 *   -c just <recipe>`; together the jobs run every recipe `just ci` runs, and nothing `just ci` does not (but the web
 *   bundle).
 * - TST-071: actionlint passes; every action of every workflow (ENV-051 too) is pinned by a major tag or a commit SHA.
 *   The other rules are about `ci.yml` only.
 * - TST-072: the Nix store comes from the `omfp` Cachix cache, which only pushes to master fill; the cargo cache is
 *   saved on master only; every job hands the dev-shell action the Cachix token.
 * - TST-073: the web bundle is built once: the e2e and desktop jobs need the `build-web` job and serve its artifact
 *   through `OMFP_WEB_BUNDLE`.
 * - TST-075: before installing Nix, the dev-shell action pools the runner's disk into a Btrfs /nix with
 *   `wimpysworld/nothing-but-nix` (Linux runners), Nix builds in /nix/build, and the cachix daemon pushes every path as
 *   it is built.
 * - TST-092: one job on a macOS runner runs `just test-desktop` and `just package` on pushes to master only.
 */
import { $ } from "bun";

interface Step {
  uses?: string;
  run?: string;
  if?: string;
  with?: Record<string, string>;
}
interface Job {
  "runs-on"?: string;
  if?: string;
  needs?: string | string[];
  env?: Record<string, string>;
  steps?: Step[];
}
interface Workflow {
  on?: { push?: { branches?: string[] }; pull_request?: unknown };
  jobs: Record<string, Job>;
}
interface Action {
  runs: { steps: Step[] };
}
interface Recipes {
  recipes: Record<string, { dependencies: { recipe: string }[] }>;
}

const WORKFLOW = ".github/workflows/ci.yml";
const SETUP = ".github/actions/dev-shell/action.yml";
const SETUP_USES = "./.github/actions/dev-shell";
const RUN = /^nix develop -c just ([\w-]+)$/;
/** The GitHub expression `${{ inner }}`. */
const expression = (inner: string) => `\${{ ${inner} }}`;
const ON_MASTER = "github.ref == 'refs/heads/master'";

const just = process.argv[2] ?? "just";
await $`actionlint`;

const workflow = Bun.YAML.parse(await Bun.file(WORKFLOW).text()) as Workflow;
const setup = Bun.YAML.parse(await Bun.file(SETUP).text()) as Action;
const { recipes } = (await $`${just} --dump --dump-format json`.json()) as Recipes;
const problems: string[] = [];

// TST-070
if (!workflow.on?.push?.branches?.includes("master") || !workflow.on || !("pull_request" in workflow.on)) {
  problems.push(`${WORKFLOW} must run on pushes to master and on pull requests`);
}
const ranBy = new Map<string, string>();
for (const [id, job] of Object.entries(workflow.jobs)) {
  const steps = job.steps ?? [];
  if (!steps.some((step) => step.uses === SETUP_USES)) {
    problems.push(`job ${id} does not use ${SETUP_USES}`);
  }
  for (const step of steps) {
    if (step.run === undefined) continue;
    const recipe = RUN.exec(step.run.trim())?.[1];
    if (recipe === undefined) {
      problems.push(`job ${id} runs \`${step.run.trim()}\`, not \`nix develop -c just <recipe>\``);
    } else if (!(recipe in recipes)) {
      problems.push(`job ${id} runs \`just ${recipe}\`, which the justfile does not define`);
    } else {
      ranBy.set(recipe, id);
    }
  }
}
const ci = recipes.ci?.dependencies.map((dependency) => dependency.recipe) ?? [];
for (const recipe of ci) {
  if (!ranBy.has(recipe)) problems.push(`\`just ci\` runs ${recipe}, no job of ${WORKFLOW} does`);
}
for (const [recipe, id] of ranBy) {
  if (recipe !== "build-web" && !ci.includes(recipe)) {
    problems.push(`job ${id} runs \`just ${recipe}\`, which \`just ci\` does not`);
  }
}

// TST-071
const PINNED = /^[^@]+@(v\d+|[0-9a-f]{40})$/;
const workflows = await Promise.all(
  [...new Bun.Glob(".github/workflows/*.{yml,yaml}").scanSync()].map(
    async (path) => Bun.YAML.parse(await Bun.file(path).text()) as Workflow,
  ),
);
const steps = [
  ...workflows.flatMap((other) => Object.values(other.jobs).flatMap((job) => job.steps ?? [])),
  ...setup.runs.steps,
];
for (const { uses } of steps) {
  if (uses !== undefined && !uses.startsWith("./") && !PINNED.test(uses)) {
    problems.push(`${uses} is pinned neither by a major tag nor by a commit SHA`);
  }
}

// TST-072
const setupStep = (action: string) => setup.runs.steps.find((step) => step.uses?.startsWith(`${action}@`));
const cachix = setupStep("cachix/cachix-action");
if (cachix?.with?.name !== "omfp") problems.push(`${SETUP} must use the omfp Cachix cache`);
if (cachix?.with?.skipPush !== expression(`!(github.event_name == 'push' && ${ON_MASTER})`)) {
  problems.push(`${SETUP} must push to Cachix on pushes to master only`);
}
if (setupStep("Swatinem/rust-cache")?.with?.["save-if"] !== expression(ON_MASTER)) {
  problems.push(`${SETUP} must save the cargo cache on master only`);
}
for (const [id, job] of Object.entries(workflow.jobs)) {
  const token = job.steps?.find((step) => step.uses === SETUP_USES)?.with?.["cachix-auth-token"];
  if (token !== expression("secrets.CACHIX_AUTH_TOKEN")) {
    problems.push(`job ${id} must hand ${SETUP_USES} the CACHIX_AUTH_TOKEN secret`);
  }
}

// TST-073
const webJob = ranBy.get("build-web");
const upload = webJob
  ? workflow.jobs[webJob]?.steps?.find((step) => step.uses?.startsWith("actions/upload-artifact@"))
  : undefined;
if (webJob === undefined || upload?.with?.path !== "ui/packages/app/dist") {
  problems.push("one job must run `just build-web` and upload ui/packages/app/dist");
}
for (const recipe of ["test-e2e", "test-desktop"]) {
  const id = ranBy.get(recipe);
  const job = id === undefined ? undefined : workflow.jobs[id];
  if (id === undefined || job === undefined) continue;
  const needs = [job.needs ?? []].flat();
  const download = job.steps?.find((step) => step.uses?.startsWith("actions/download-artifact@"));
  if (
    webJob === undefined ||
    !needs.includes(webJob) ||
    job.env?.OMFP_WEB_BUNDLE === undefined ||
    download?.with?.name !== upload?.with?.name ||
    download?.with?.path !== expression("env.OMFP_WEB_BUNDLE")
  ) {
    problems.push(`job ${id} must serve the bundle of job ${webJob ?? "?"} through OMFP_WEB_BUNDLE`);
  }
}

// TST-075
const setupIndex = (action: string) => setup.runs.steps.findIndex((step) => step.uses?.startsWith(`${action}@`));
const pool = setupIndex("wimpysworld/nothing-but-nix");
const install = setupIndex("cachix/install-nix-action");
if (pool === -1 || install === -1 || pool > install) {
  problems.push(`${SETUP} must run wimpysworld/nothing-but-nix before cachix/install-nix-action`);
}
if (!/^build-dir = \/nix\/build$/m.test(setup.runs.steps[install]?.with?.extra_nix_config ?? "")) {
  problems.push(`${SETUP} must make Nix build in /nix/build (extra_nix_config: build-dir = /nix/build)`);
}
if (cachix?.with?.useDaemon === "false") {
  problems.push(`${SETUP} must keep the cachix daemon, which pushes what a failing job built`);
}

// TST-092
const MACOS_RECIPES = ["test-desktop", "package"];
const macos = Object.entries(workflow.jobs).filter(([, job]) => /^macos-\d+$/.test(job["runs-on"] ?? ""));
if (
  !macos.some(([, job]) => {
    const recipes = (job.steps ?? []).map((step) => RUN.exec(step.run?.trim() ?? "")?.[1]);
    return job.if === "github.event_name == 'push'" && MACOS_RECIPES.every((recipe) => recipes.includes(recipe));
  })
) {
  problems.push(`one job on a macOS runner must run ${MACOS_RECIPES.join(" and ")} on pushes to master only`);
}

if (problems.length > 0) {
  console.error([...new Set(problems)].join("\n"));
  process.exit(1);
}
