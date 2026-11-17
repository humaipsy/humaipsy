# BRAND.md — the HumaiPsy mark

The design record for `assets/logo/`. [`README.md`](README.md) is the quick guide;
[`MANIFEST.toml`](MANIFEST.toml) is the machine-readable one — every palette value and the
SHA-256 of every file here.

## What the mark says

A HumaiPsy session is one participant talking to two agents whose conditions are
counterbalanced. The mark draws exactly that, and nothing else:

- **one speaker** — a head and shoulders, peach;
- **two agents** — two faces, each beside its own reply, in two tints of one blue;
- **three turns** — one human turn, two agent turns, stacked in the order they happen.

The two blues are not two brands. They are one colour at two values, because
Cooperative and Argumentative are two conditions of the same instrument, not two
products. A future third condition would need a third tint of the same blue, and the
mark's rows are the only thing that would change.

## Construction

The mark is drawn on a **32-unit grid**, so every coordinate in the source SVGs is a
number a reviewer can check. View box `0 0 32 32`; the mark's own box is
`x 4–28, y 5.25–26` — 24 × 20.75 units, flush on both edges.

| Element | Geometry |
| --- | --- |
| Rows | bubbles at `y 6–11`, `13.5–18.5`, `21–26` — 5 tall, 2.5 apart |
| Human bubble | `x 11–28`, `y 6–11`, `rx 2.5` |
| Agent bubbles | `x 4–22`, `y 13.5–18.5` and `21–26`, `rx 2.5` |
| Detail bars | 1.2 tall, `rx 0.6` — `y 7.9`, `15.4`, `22.9` |
| Human head | circle, centre `(6.9, 6.9)`, `r 1.65` |
| Human shoulders | half-ellipse `x 4–9.8`, bottom `y 11.8`, `ry 2.75` (apex `y 9.05`) |
| Agent face | `4.2 × 4.2`, `rx 1.2` — `x 23.8–28`, `y 13.9–18.1` and `21.4–25.6` |
| Antenna | stem `0.7 × 1.1` at `x 25.55`, knob `r 0.5` at `(25.9, 12.75)` / `(25.9, 20.25)` |
| Eyes | `0.95 × 0.95`, `rx 0.24` — `x 24.55` and `26.3`, at `y 15.53` / `23.03` |

Two alignments hold the drawing together, and both are the reason the mark looks drawn
rather than assembled:

1. **The head and the shoulders share one centre line, `x 6.9`.** This is the failure the
   first version of this mark had — a head sitting a unit and a half off its shoulders —
   and it is the one thing to measure if the mark is ever redrawn.
2. **Every row is centred on its own face.** A face is 4.2 tall inside a 5-unit row, so
   it carries 0.4 of breathing room above and below; the antenna's knob then sits exactly
   1.25 from the bubble above it. Three rows at one pitch, no exceptions.

The detail bars are set to one side of each bubble and never centred, so a row reads as a
message that starts where a message starts.

## The tile

The application icon is the mark on a **32-unit square with `rx 8.5`** (272 px at 1024),
filled with a vertical gradient:

| Stop | Value | Token |
| --- | --- | --- |
| top | `#363A4F` | Catppuccin Macchiato `surface0` |
| bottom | `#24273A` | Catppuccin Macchiato `base` |

The gradient runs top-light to bottom-dark for the same reason a window does: the mark is
lit from above, and the tile has a top and a bottom. The tile's corners are the only
transparency in `humaipsy-app.svg`.

An icon has to hold its own against whatever is behind it. A bare colour mark on a dark
taskbar loses its navy nothing but gains no silhouette; the tile is what gives the eye an
edge to find at 16 px.

## Colour

| Token | Value | Role | Where it comes from |
| --- | --- | --- | --- |
| Peach | `#F5A97F` | the human: head, shoulders, bubble | Catppuccin Macchiato `peach` — the exact value `humaipsy/src/ui/theme.rs` paints with |
| Sapphire | `#7DC4E4` | the first agent: face, antenna, bubble | Catppuccin Macchiato `sapphire` |
| Sapphire deep | `#5E9FC7` | the second agent: the same shapes | sapphire, deepened — this mark's own value |
| Parchment | `#F4F1EA` | the detail bars | this mark's own value |
| Ink | `#24273A` | eyes, tile | Macchiato `base` |
| Tile | `#363A4F → #24273A` | the application tile | Macchiato `surface0 → base` |

Peach and the two tile navies are not colours invented for a picture: they are the values
`theme::MACCHIATO` already paints the interface with, so the icon and the window cannot
drift apart. Sapphire is the published Macchiato token of that name; `theme.rs`'s palette
struct does not carry it today, so it exists in the mark only. Sapphire deep and parchment
are the two values the mark adds, and they are the only two it may add.

One colour per meaning. Peach never appears on an agent, a blue never appears on the human,
and there is no third hue anywhere in the family.

## Contrast

Measured, sRGB, WCAG 2.1 relative luminance.

| Pair | Ratio | Use |
| --- | --- | --- |
| Peach on tile | 7.62:1 | mark on the application tile |
| Sapphire on tile | 7.63:1 | mark on the application tile |
| Sapphire deep on tile | 5.09:1 | mark on the application tile |
| Ink on sapphire | 7.63:1 | the eyes |
| Ink on sapphire deep | 5.09:1 | the eyes |
| White on ink | 14.72:1 | the one-ink mark on a dark ground |
| Parchment on peach | 1.71:1 | detail bars, non-text |
| Parchment on sapphire deep | 2.56:1 | detail bars, non-text |

The detail bars are **not text**. They stand for a line of message, not for words, and
nobody is meant to read them; 1.71:1 is why they read as a mark instead of as a caption.
Do not raise their contrast by making them pure white — the bar would start looking like
a letter. Nothing in the mark is held to a text ratio, because nothing in it is text.

**Greyscale.** Peach (L 0.493) and sapphire (L 0.494) are the same value to within a
rounding error, so a greyscale reproduction keeps the structure — one speaker, two replies,
three turns — and loses only which of the first two rows is whose. The two agents stay
distinct from each other (0.494 against 0.313), and the human's row keeps its avatar, so
the drawing survives one-ink printing even though the palette does not. Where the colours
matter, print in colour or use `humaipsy-white.svg`.

## Clear space and minimum sizes

Clear space is **2.5 units** — half a bubble height — measured from the mark's own box on
all four sides. At the sizes this mark is used at, that is what keeps a neighbouring edge
from touching a bubble.

| Form | Specified from | Below that |
| --- | --- | --- |
| `humaipsy.svg` (bare mark) | 32 px | the bars close up and the antennae become specks; the three rows still read |
| `humaipsy-app.svg` (tile) | 16 px | at 16 px the mark stops being legible and becomes a silhouette — one peach band over two blue ones — which is all any 16 px icon is |

Below 32 px, use the tile. That is the whole reason the tile exists.

## The three files

| Family | For |
| --- | --- |
| `humaipsy.svg`, `humaipsy-<size>.png` | Transparent, full colour. Light or dark ground, any size from 32 px. |
| `humaipsy-white.svg`, `humaipsy-white-<size>.png` | One ink: every shape white, the bars and eyes knocked out of it, so the mark takes the colour of what is behind it. Dark grounds, single-colour print, tray icons. |
| `humaipsy-app.svg`, `humaipsy-app-<size>.png` | The tile. OS icons, the window bar, store listings. |
| `humaipsy-app.rgba` | 64 × 64 unmultiplied RGBA for `egui::IconData`, embedded with `include_bytes!`. |
| `humaipsy-white.rgba` | The same 64 × 64 as a bare white mark, for a tray. |

## Rules

- **Do not** rotate, skew, outline, emboss or add a drop shadow to the mark.
- **Do not** add a gradient to the bubbles or the avatar. The tile has a gradient; the
  mark does not.
- **Do not** recolour one agent without the other. They are a condition pair.
- **Do not** introduce a hue that is not in the table above — in particular, do not reach
  for the website's ink and teal accent, which are a different system for a different
  surface.
- **Do not** change the mark's aspect. It is 24 × 20.75 units.
- **Do not** set the colour mark on a photograph or a mid-tone it does not read on; use the
  tile or the white mark.
- **Do** keep the mark's own box flush — `x 4–28` — when placing it, and add the clear
  space outside that box rather than shrinking the mark inside it.

## Regenerating

The three SVGs are the source of truth. The PNGs and the two `.rgba` files are generated:

1. Render each SVG at its exact pixel size on a transparent canvas — `chrome --headless
   --screenshot --default-background-color=00000000 --window-size=<n>,<n>` is what produced
   the set in this folder. Render each size from the SVG rather than downscaling a master:
   at 16 and 24 px that is the difference between a crisp bar and a grey smear.
2. For the tile, keep the rounded square; only the corners are transparent.
3. Decode `humaipsy-app-64.png` to unmultiplied RGBA and write the 16,384 raw bytes to
   `humaipsy-app.rgba`; decode `humaipsy-white-64.png` the same way to
   `humaipsy-white.rgba`.
4. Regenerate `contact-sheet.png` and `MANIFEST.toml` so the sheet and the digests match
   what is on disk.

`contact-sheet.png` is a 1600 × 1000 sheet on the ink ground: the three variants at 200 px,
the tile at six sizes, the palette, the contrast pairs and the construction note. It is
generated from a throwaway HTML page that references the files in this folder — it is a
picture of them, not an independent drawing.

## Licence

AGPL-3.0-only, like the rest of the repository — see [`../../LICENSE`](../../LICENSE). The
mark is original HumaiPsy work and contains no third-party asset.
