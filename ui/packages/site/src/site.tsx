import { Badge, Button, Card, Icon, IconButton, LogoMark, Wordmark } from "@omfp/kit";
import { type ReactNode, useEffect, useRef, useState } from "react";

const REPOSITORY = "https://github.com/aciceri/omfp";
const RELEASES = `${REPOSITORY}/releases/latest`;
const OVERVIEW = `${REPOSITORY}/blob/master/docs/overview.md`;

type Scheme = "dark" | "light";

interface SiteProps {
  videoUrl?: string;
}

interface ActionLinkProps {
  children: ReactNode;
  href: string;
  primary?: boolean;
}

function ActionLink({ children, href, primary = false }: ActionLinkProps) {
  return (
    <a className="omfp-button site-action" data-variant={primary ? "primary" : "secondary"} href={href}>
      {children}
      <Icon name="external-link" />
    </a>
  );
}

function Screenshot({ scheme }: { scheme: Scheme }) {
  return (
    <div
      className="site-window"
      onPointerMove={(event) => {
        const bounds = event.currentTarget.getBoundingClientRect();
        const horizontal = (event.clientX - bounds.left) / bounds.width - 0.5;
        const vertical = (event.clientY - bounds.top) / bounds.height - 0.5;
        event.currentTarget.style.setProperty("--site-tilt-x", `${-vertical * 4}deg`);
        event.currentTarget.style.setProperty("--site-tilt-y", `${horizontal * 6}deg`);
      }}
      onPointerLeave={(event) => {
        event.currentTarget.style.removeProperty("--site-tilt-x");
        event.currentTarget.style.removeProperty("--site-tilt-y");
      }}
    >
      <div className="site-window__bar" aria-hidden="true">
        <LogoMark className="site-window__mark" />
        <span>release room</span>
        <span className="site-window__status">● 4 agents active</span>
      </div>
      <picture>
        <img
          src={scheme === "dark" ? "./screenshot-dark.png" : "./screenshot-light.png"}
          alt="omfp showing a running agent thread and its workspace"
        />
      </picture>
    </div>
  );
}

const SHOWCASE_SLIDES = [
  {
    src: "./carousel/01-release-room.webp",
    title: "One release, many agents",
    description: "Coordinate agent tasks, CI checks, rollout health and the conversation in one release room.",
    alt: "Release coordination workspace with conversation, agent tasks, CI checks and rollout health.",
  },
  {
    src: "./carousel/05-film-restoration.webp",
    title: "Make room for the image",
    description: "Let the material lead: restoration preview, scene queue, analysis and a compact agent conversation.",
    alt: "Film restoration workspace with archival before-and-after imagery, a scene queue and agent conversation.",
  },
  {
    src: "./carousel/02-performance-lab.webp",
    title: "A workspace for the investigation",
    description: "Keep flamegraphs, benchmark comparisons, findings and the next experiment in the same view.",
    alt: "Performance investigation workspace with a flamegraph, benchmark comparisons and agent conversation.",
  },
  {
    src: "./carousel/04-robotics.webp",
    title: "Follow the data into the physical world",
    description: "Compare trajectories, inspect telemetry and keep the agent beside the evidence.",
    alt: "Robot navigation workspace comparing warehouse trajectories, agent conversation and telemetry.",
  },
  {
    src: "./carousel/03-product-room.webp",
    title: "The launch, not another backlog",
    description: "Shape the room around dependencies, customer feedback, decisions and the work moving through them.",
    alt: "Product launch workspace combining a kanban, dependencies, customer feedback and agent conversation.",
  },
  {
    src: "./carousel/06-astronomy.webp",
    title: "A different kind of night shift",
    description: "Plan observing windows, exposure sequences and targets without leaving the conversation.",
    alt: "Observatory workspace with a star map, observing windows, exposure sequences and agent conversation.",
  },
] as const;

const SHOWCASE_INTERVAL_MS = 8_000;

function Showcase() {
  const rootRef = useRef<HTMLElement>(null);
  const swipeStart = useRef<number | undefined>(undefined);
  const [index, setIndex] = useState(0);
  const [visible, setVisible] = useState(false);
  const [paused, setPaused] = useState(false);
  const [interacted, setInteracted] = useState(false);

  useEffect(() => {
    const root = rootRef.current;
    if (root === null || typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(([entry]) => setVisible(entry?.isIntersecting === true), {
      threshold: 0.35,
    });
    observer.observe(root);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const reduced = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
    if (!visible || paused || interacted || reduced) return;
    const timer = window.setInterval(
      () => setIndex((current) => (current + 1) % SHOWCASE_SLIDES.length),
      SHOWCASE_INTERVAL_MS,
    );
    return () => window.clearInterval(timer);
  }, [interacted, paused, visible]);

  const select = (next: number) => {
    setInteracted(true);
    setIndex((next + SHOWCASE_SLIDES.length) % SHOWCASE_SLIDES.length);
  };
  const current = SHOWCASE_SLIDES[index] ?? SHOWCASE_SLIDES[0];
  const autoplaying = visible && !paused && !interacted;

  return (
    <section
      ref={rootRef}
      className="site-showcase"
      id="showcase"
      aria-labelledby="showcase-title"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
      onFocusCapture={() => setPaused(true)}
      onBlurCapture={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setPaused(false);
      }}
    >
      <header className="site-showcase__head">
        <div>
          <p className="site-kicker">the deeply customizable orchestrator for hackers</p>
          <h1 id="showcase-title">
            Every workflow is different.
            <br />
            Why should the interface be the same?
          </h1>
        </div>
      </header>

      <fieldset
        className="site-showcase__stage"
        data-slide={index}
        data-testid="showcase-stage"
        data-autoplay={autoplaying}
        onKeyDown={(event) => {
          if (event.key === "ArrowLeft") select(index - 1);
          if (event.key === "ArrowRight") select(index + 1);
        }}
        onPointerDown={(event) => {
          swipeStart.current = event.clientX;
        }}
        onPointerUp={(event) => {
          const start = swipeStart.current;
          swipeStart.current = undefined;
          if (start === undefined || Math.abs(event.clientX - start) < 50) return;
          select(index + (event.clientX < start ? 1 : -1));
        }}
      >
        <legend className="omfp-visually-hidden">Agent-built workspaces</legend>
        <figure className="site-showcase__current">
          <img key={current.src} src={current.src} alt={current.alt} fetchPriority={index === 0 ? "high" : "auto"} />
        </figure>
        <IconButton
          className="site-showcase__arrow"
          data-direction="previous"
          label="Previous workspace"
          icon={<span aria-hidden="true">←</span>}
          onPress={() => select(index - 1)}
        />
        <IconButton
          className="site-showcase__arrow"
          data-direction="next"
          label="Next workspace"
          icon={<span aria-hidden="true">→</span>}
          onPress={() => select(index + 1)}
        />
      </fieldset>

      <div className="site-showcase__caption" aria-live="polite">
        <div>
          <Badge tone="primary">agent-built workspace</Badge>
          <h2>{current.title}</h2>
          <p>{current.description}</p>
        </div>
        <span className="site-showcase__count">
          {String(index + 1).padStart(2, "0")} / {String(SHOWCASE_SLIDES.length).padStart(2, "0")}
        </span>
      </div>

      <div className="site-showcase__tabs" role="toolbar" aria-label="Choose a workspace">
        {SHOWCASE_SLIDES.map((slide, slideIndex) => (
          <Button
            key={slide.src}
            size="sm"
            variant="quiet"
            aria-label={`Show ${slide.title}`}
            aria-pressed={slideIndex === index}
            onPress={() => select(slideIndex)}
          >
            {String(slideIndex + 1).padStart(2, "0")}
          </Button>
        ))}
        <span key={index} className="site-showcase__progress" data-running={autoplaying} aria-hidden="true" />
      </div>
    </section>
  );
}

function Film({ videoUrl }: { videoUrl: string }) {
  const videoRef = useRef<HTMLVideoElement>(null);

  useEffect(() => {
    const video = videoRef.current;
    if (video === null) return;
    video.muted = true;
    const start = () => {
      const started = video.play();
      if (started !== undefined) void started.catch(() => undefined);
    };
    if (typeof IntersectionObserver === "undefined") {
      start();
      return;
    }
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting === true) {
          start();
        } else {
          video.pause();
        }
      },
      { threshold: 0.35 },
    );
    observer.observe(video);
    return () => observer.disconnect();
  }, []);

  return (
    <section className="site-section site-film" id="film" aria-labelledby="film-title">
      <div className="site-film__frame">
        <video ref={videoRef} autoPlay muted controls loop playsInline preload="auto" poster="./screenshot-dark.png">
          <source src={videoUrl} type="video/mp4" />
        </video>
      </div>
      <div className="site-section__copy">
        <p className="site-kicker">one-minute product demo</p>
        <h2 id="film-title">Your workflow is not standard. Your interface should not be either.</h2>
        <p>
          Watch agents connect the tools a release needs, split the work, review a patch, and reshape the workspace
          around the job.
        </p>
      </div>
    </section>
  );
}

const FEATURES = [
  {
    icon: "folder-git-2" as const,
    title: "A workspace is just a directory",
    body: "A workspace is simply a directory. You might keep repositories, worktrees, UI and widget declarations, skills, or background-job definitions there—or add your own conventions and tooling, such as encrypted-secret support. These are examples, not requirements. Threads work inside it and can start more threads; the directory itself is the only structure omfp imposes.",
  },
  {
    icon: "panel-right" as const,
    title: "Ask the agent to reshape the UI",
    body: "Change a workspace view live by asking the agent. Under the hood, every view is made of React widgets. Even the default components are editable—a starting canvas, not a fixed product interface.",
  },
  {
    icon: "globe" as const,
    title: "One daemon, many windows",
    body: "A headless local daemon keeps the work running. Connect from a browser, the mobile PWA, or the Tauri app on macOS and Linux. Open several windows on the same daemon and arrange them around your workflow and window manager.",
  },
  {
    icon: "command" as const,
    title: "Mouse-friendly. Keyboard-first.",
    body: "The default widgets work well with a mouse, but every surface is designed for serious keyboard use. The default interaction model borrows from modal editors such as Helix and Vim.",
  },
];

export function Site({ videoUrl }: SiteProps) {
  const [scheme, setScheme] = useState<Scheme>("dark");
  useEffect(() => {
    document.documentElement.dataset.colorScheme = scheme;
  }, [scheme]);
  const switchScheme = () => {
    const next: Scheme = scheme === "dark" ? "light" : "dark";
    document.documentElement.dataset.colorScheme = next;
    setScheme(next);
  };

  return (
    <div className="site-shell">
      <header className="site-nav">
        <a className="site-nav__brand" href="#top" aria-label="omfp home">
          <Wordmark />
        </a>
        <nav aria-label="Main navigation">
          {videoUrl !== undefined && <a href="#film">film</a>}
          <a href="#why">why omfp</a>
          <a href={OVERVIEW}>how it works</a>
          <a href={REPOSITORY}>source</a>
          <IconButton label="Switch color scheme" icon={<span aria-hidden="true">◐</span>} onPress={switchScheme} />
        </nav>
      </header>

      <main id="top">
        <Showcase />
        {videoUrl !== undefined && <Film videoUrl={videoUrl} />}

        <section className="site-hero" aria-labelledby="hero-title">
          <div className="site-hero__copy">
            <p className="site-kicker">a workspace your agents can reshape</p>
            <h2 id="hero-title">Many agents. One workspace. Your interface.</h2>
            <p className="site-hero__lead">
              omfp is a browser and desktop client for oh-my-pi: parallel conversations, persistent workspaces, and a UI
              the agent can rewrite while it runs.
            </p>
            <div className="site-actions">
              <ActionLink href={RELEASES} primary>
                Get omfp
              </ActionLink>
              <ActionLink href={REPOSITORY}>View source</ActionLink>
            </div>
            <p className="site-hero__note">Bring your ChatGPT or Claude plan, or any API key oh-my-pi supports.</p>
          </div>
          <Screenshot scheme={scheme} />
        </section>
        <section className="site-section" id="why" aria-labelledby="why-title">
          <div className="site-section__copy">
            <p className="site-kicker">stable core, replaceable interface</p>
            <h2 id="why-title">
              A small core.
              <br />
              The rest is yours.
            </h2>
            <p>
              omfp keeps orchestration, persistence, and remote access boring. Everything specific to the way you work
              can stay visible, editable, and disposable.
            </p>
          </div>
          <div className="site-feature-grid">
            {FEATURES.map((feature) => (
              <Card
                className="site-feature"
                key={feature.title}
                title={
                  <span className="site-feature__title">
                    <Icon name={feature.icon} />
                    {feature.title}
                  </span>
                }
              >
                <p>{feature.body}</p>
              </Card>
            ))}
          </div>
        </section>

        <section className="site-section site-interface" id="interface" aria-labelledby="interface-title">
          <div className="site-interface__demo">
            <div className="site-interface__command">▸ “Add a release panel with checks and deploy.”</div>
            <Card title="╭─ release / v0.1.0" tone="success">
              <div className="site-check">
                <span>●</span> Linux package <strong>passed</strong>
              </div>
              <div className="site-check">
                <span>●</span> macOS package <strong>passed</strong>
              </div>
              <div className="site-check">
                <span>●</span> contract suite <strong>passed</strong>
              </div>
              <div className="site-deploy">
                deploy production <span>enter</span>
              </div>
            </Card>
          </div>
          <div className="site-section__copy">
            <p className="site-kicker">emacs philosophy at heart</p>
            <h2 id="interface-title">Change the tool from inside the tool.</h2>
            <p>
              Tell the agent what the task needs. It writes and mounts a React view using the same SDK and Crush
              components as the built-ins. Manual editing stays available when you want it; no app rebuild.
            </p>
            <ActionLink href={OVERVIEW}>See how it works</ActionLink>
          </div>
        </section>

        <section className="site-cta" aria-labelledby="cta-title">
          <LogoMark aria-hidden="true" />
          <div>
            <p className="site-kicker">early, open, changing fast</p>
            <h2 id="cta-title">Make the workspace fit the work.</h2>
          </div>
          <ActionLink href={RELEASES} primary>
            Download the latest release
          </ActionLink>
        </section>
      </main>

      <footer>
        <Wordmark className="site-footer__logo" aria-hidden="true" />
        <span>The f stands for fabulous, right?</span>
        <a className="site-footer__link" href={`${REPOSITORY}/blob/master/LICENSE`}>
          GPL-3.0-or-later
        </a>
      </footer>
    </div>
  );
}
