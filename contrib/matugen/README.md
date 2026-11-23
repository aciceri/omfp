# omfp theme from matugen

[matugen](https://github.com/InioX/matugen) generates a Material 3 color scheme (from a wallpaper or a color);
`omfp.css` here is a matugen template that turns it into an omfp theme: CSS overriding the `--omfp-*` color tokens
for light and dark (UI-091). The theme is global to omfp and kept by the daemon, so every window (browser or desktop,
local or remote) picks it up live while the `matugen` theme is selected.

## Setup

1. Copy `omfp.css` to `~/.config/matugen/templates/omfp.css`.
2. Merge `config.toml` into `~/.config/matugen/config.toml` (the `[config.custom_colors]` entries are required: the
   template uses `success` and `warning`).
3. Run matugen, e.g. `matugen image ~/wallpaper.png` or `matugen color hex "#6750a4"`.
4. Select the `matugen` theme once: `:theme matugen` in a window (or `:set theme=matugen`, or `theme = "matugen"`
   under `[ui]` in `~/.config/omfp/config.toml`). With any other theme the daemon keeps the generated theme but the
   windows do not show it; selecting `matugen` again shows the latest one. Without a generated theme, `matugen` is
   Synth.

Shells that run matugen for you and add your templates to theirs (DankMaterialShell and its forks, with "Run user
templates") use absolute paths in `input_path`/`output_path` and do not pass `[config.custom_colors]` to user templates:
there, delete the `--omfp-success`, `--omfp-on-success`, `--omfp-warning` and `--omfp-on-warning` lines from the
template (omfp keeps its built-in status colors) or the whole template fails to render.

Where the theme goes:

- **Daemon on this machine**: the template's `output_path` is `~/.config/omfp/theme.css`
  (`$XDG_CONFIG_HOME/omfp/theme.css`), the file the daemon follows on its host (`omfp daemon --theme-file` changes
  it). Every change is pushed to all windows.
- **Remote daemon**: uncomment the `post_hook`, which runs `omfp theme set <file> --daemon <url> --token <token>`
  after each generation (the daemon stores it like `theme.set`; `--daemon`/`--token` default to the local daemon).

`omfp theme set` works with any CSS file, so a hand-written theme overriding a few tokens is fine too; it shows with
the `matugen` theme as well. An empty theme (an empty file, or `theme.set` with `css: null`) makes `matugen` show
Synth.

## How it overrides

With the `matugen` theme the windows put the theme over Synth's tokens, which live in `@layer omfp-tokens`
(`ui/packages/kit/src/tokens.css`); the template is unlayered, so its declarations always win. Like the built-in
themes it is one `:root` rule whose roles are `light-dark({{colors.<name>.light.hex}}, {{colors.<name>.dark.hex}})`,
so the color-scheme switch (`:theme light|dark|system`) keeps working: omfp sets `color-scheme` on the root (the
system's, or the forced one), which picks the side. A hand-written theme may also put its dark values under
`@media (prefers-color-scheme: dark) { :root:not([data-color-scheme="light"]) }` and
`:root[data-color-scheme="dark"]` instead.

Only the roles are needed: gradients, hover, selection, focus, diff and status colors are derived from them in the
built-in tokens (UI-135, UI-136), so they follow the theme. Fonts, sizes, spacing, radii and density are not part of the
template; add declarations for `--omfp-font-prose`, `--omfp-font-code`, … to the generated `:root` rule (or the
template) to change them. The overall text size is the `[ui] font_size` setting of the config file (root font size in
px, default 17; every rem-based token scales with it).

## Token mapping

Every omfp color token and the matugen color it takes (`{{colors.<matugen>.light.hex}}` in light,
`.dark.hex` in dark):

| omfp token | matugen color |
| --- | --- |
| `--omfp-primary` | `primary` |
| `--omfp-on-primary` | `on_primary` |
| `--omfp-primary-container` | `primary_container` |
| `--omfp-on-primary-container` | `on_primary_container` |
| `--omfp-secondary` | `secondary` |
| `--omfp-on-secondary` | `on_secondary` |
| `--omfp-secondary-container` | `secondary_container` |
| `--omfp-on-secondary-container` | `on_secondary_container` |
| `--omfp-tertiary` | `tertiary` |
| `--omfp-on-tertiary` | `on_tertiary` |
| `--omfp-tertiary-container` | `tertiary_container` |
| `--omfp-on-tertiary-container` | `on_tertiary_container` |
| `--omfp-error` | `error` |
| `--omfp-on-error` | `on_error` |
| `--omfp-error-container` | `error_container` |
| `--omfp-on-error-container` | `on_error_container` |
| `--omfp-surface` | `surface` |
| `--omfp-on-surface` | `on_surface` |
| `--omfp-surface-variant` | `surface_variant` |
| `--omfp-on-surface-variant` | `on_surface_variant` |
| `--omfp-surface-container-lowest` | `surface_container_lowest` |
| `--omfp-surface-container-low` | `surface_container_low` |
| `--omfp-surface-container` | `surface_container` |
| `--omfp-surface-container-high` | `surface_container_high` |
| `--omfp-surface-container-highest` | `surface_container_highest` |
| `--omfp-surface-dim` | `surface_dim` |
| `--omfp-surface-bright` | `surface_bright` |
| `--omfp-inverse-surface` | `inverse_surface` |
| `--omfp-inverse-on-surface` | `inverse_on_surface` |
| `--omfp-inverse-primary` | `inverse_primary` |
| `--omfp-outline` | `outline` |
| `--omfp-outline-variant` | `outline_variant` |
| `--omfp-shadow` | `shadow` |
| `--omfp-scrim` | `scrim` |
| `--omfp-success` | `success` |
| `--omfp-on-success` | `on_success` |
| `--omfp-warning` | `warning` |
| `--omfp-on-warning` | `on_warning` |

`success`/`on_success` and `warning`/`on_warning` come from `[config.custom_colors]` (matugen generates the `on_*`
and `*_container` variants of each custom color).

matugen colors without an omfp token (not used yet): `primary_fixed`, `primary_fixed_dim`, `on_primary_fixed`,
`on_primary_fixed_variant` and the same `*_fixed*` set for `secondary` and `tertiary`; `background` and
`on_background` (omfp uses `surface`/`on_surface`, as Material 3 does now); `source_color`; the `*_container`
variants of the custom colors.
