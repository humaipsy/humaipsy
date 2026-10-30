# DESIGN.md — the HumaiPsy website

The durable visual system for `docs/`, which [pages.yml](.github/workflows/pages.yml)
publishes to the project's GitHub Pages site. This document is about the *website*
only; the desktop application draws its own interface in `humaipsy/src/ui/theme.rs`
and the two are deliberately not the same system.

## What the surface is

One page, one job: convince a researcher that HumaiPsy is a legitimate instrument for
running a study, and send them to a release. Mode: **persuade**. It ships as static
HTML and CSS with no build step and **no JavaScript at all** — `docs/index.html`,
`docs/style.css`, three self-hosted woff2 files. Deploying is a push to `main` that
touches `docs/`.

## The world

A printed **specification sheet for a lab instrument**. One ink, hairline rules, line
work only, and fields a reviewer could check. The type stays quiet in the left margin;
the design figure carries the argument instead of adjectives. Anything that would look
at home on a software landing page (a feature-card grid, a kicker above a heading, a
gradient, a glowing accent) is out of the world by construction.

Chosen over four alternates that were also viable — journal figure plate, optical
bench, preregistration form, strip-chart recorder. Do not blend them in.

## Colour

One light. There is no dark theme, deliberately: the page is a document, it is read
beside a bench and it is printed, and a `prefers-color-scheme` branch hands the
decision to the operating system. `color-scheme: light` is set so browser controls
follow.

| token | value | use |
| --- | --- | --- |
| `--ground` | `#ffffff` | the sheet |
| `--ink` | `#15191b` | body, display, solid figure fill |
| `--ink-2` | `#4c5559` | secondary prose, axis labels |
| `--ink-3` | `#666f74` | field labels, ticks, footer |
| `--rule` | `#dfe3e4` | hairlines between fields and rows |
| `--rule-2` | `#c2c9cb` | the stronger rule under the title block |
| `--accent` | `#1c4b63` | the running-trial marker and interface links only |
| `--accent-soft` | `#edf2f4` | text selection |

The accent is the only non-grey on the page and it carries state, not decoration. If a
new element reaches for a colour that is not in this table, it is reaching for the
wrong thing — emphasis here comes from weight, size and rules.

## Type

Both faces are self-hosted in `docs/fonts/` and loaded with `@font-face`. No hosted
font stylesheet, ever: the page must work with no network, which is the same promise
the product makes.

- **Public Sans** (variable, `100 900`, latin subset) for text, display and interface.
  A government-forms grotesque: sober and unglamorous, the register of a protocol
  rather than of a product launch. Display is 600 at
  `clamp(30px, 1.4rem + 2.8vw, 50px)`, `letter-spacing: -0.027em`, `line-height: 1.07`.
- **Courier Prime** for machine data only — output filenames, figure annotations,
  licence strings, numerals inside a figure caption. It stands in for what a filled-in
  sheet types. It is never used to decorate prose.
- `font-variant-numeric: tabular-nums` is set globally: this is a page of measurements.

Body is 16.5px/1.6; prose measures are capped around 56–62ch.

## Layout

- Shell `1120px`, side padding `clamp(20px, 4.5vw, 56px)`.
- Below the title block the page is a **field grid**: a `188px` label column with a
  hairline on its right, then the content. Each field is separated by a hairline and
  inset by `padding-block`. The labels are `h2` elements in Courier Prime, uppercase,
  12px — they are the document's section headings, and they are not kickers: there is
  no second, larger heading beneath them.
- The title block is full width, so its left edge and the field labels' left edge
  deliberately disagree by the label column.
- At `760px` the field grid collapses to one column and the label becomes a
  rule-prefixed header; the figure swaps plates (below).

## The figure

`Fig. 1` is the page's whole argument: a 2 × 2 design matrix (`AGENT A` / `AGENT B` as
rows, `COND X` / `COND Y` as columns) beside one participant's twelve-trial sequence,
with the running trial bracketed as `04 / 12`.

- Encodings: **solid fill = COND X**, **hatch = COND Y**, **position = trial order**.
  One channel per meaning; do not add a second colour to distinguish conditions.
- Conditions are distinguished by pattern, not hue, so the figure survives greyscale
  printing and colour-vision deficiency.
- It exists as **two inline SVG plates** — `--wide` and `--compact` — swapped at
  `760px`. An SVG scales its own labels, so a single plate rendered at 0.45× would
  have produced 5px figure text on a phone. **Edit both plates**; a change to one that
  is not mirrored in the other is a bug you will only see on a phone.

## Refusals

- No shadows, no gradients, no glow, no rounded cards. Depth is rules and whitespace.
- No kicker or eyebrow above a heading; the heading carries its own weight.
- No numbered sections unless the sequence is information. The procedure keeps `01`,
  `02`, `03` because a procedure's order is the content; the old feature cards did not.
- No monospace as a costume for "technical". It is for data and machine identifiers.
- No second accent colour, and no colour that is not in the table above.
- The page ships no JavaScript; do not add a script tag for an effect.

## Verifying a change

Render it, do not eyeball the source. `fe_render` on `docs/index.html` at `1440x900`
and `390x844` reports measured contrast, tap targets, overflow, line length and small
type — the current page is at zero findings on both, and `fe_detect` on `docs/` is
also clean. A change that adds a finding needs either a fix or a written reason.
