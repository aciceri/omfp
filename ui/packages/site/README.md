# omfp project website

The single-page project site shares `@omfp/kit` with the app and builds independently into `dist/`.

The document starts in the dark color scheme even when the operating system is light, so the dark workspace captures and film remain visually integrated. The navigation control still switches the complete page to the light scheme.

```sh
just build-site
```

## Showcase carousel

The first section bundles six optimized WebP workspace views from `public/carousel/`. Keep Release room first; the remaining order is film restoration, performance lab, robotics, product room and astronomy. The carousel advances every eight seconds only while visible, pauses on hover or focus, and stops after manual interaction.

## Promotional film

Keep the editable film project in `contrib/promo/`; keep generated frames, audio and MP4 files out of Git. Publish the approved MP4 as a GitHub release asset so it has a stable URL and does not inflate the repository history.

Build the site with that direct MP4 URL:

```sh
OMFP_SITE_VIDEO_URL=https://github.com/aciceri/omfp/releases/download/film-v1/omfp-release-story.mp4 just build-site
```

The film section is omitted when the variable is empty, so a missing asset never produces a broken player. GitHub Pages reads the same value from the repository variable `OMFP_SITE_VIDEO_URL`. The README's film link and screenshot open `https://aciceri.github.io/omfp/#film`; the website is the canonical presentation while the `film-v1` release asset is the canonical file.

The soundtrack credit and license links from `contrib/promo/README.md` must also be present in the release or video-platform description.
