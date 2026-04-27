# Justified gallery layout

## Problem

The current gallery uses a fixed row height with `flex: 0 0 auto` — photos sit at their
natural widths and rows are not filled edge-to-edge. This avoids cropping but leaves ragged
right edges, most visibly when a small number of photos with very different aspect ratios
(e.g. one landscape and one portrait) share a row.

## Idea

Implement a justified gallery: every row is scaled so it fills the full container width
exactly, and every photo in a row shares the same height. This is the layout used by
Google Photos and Flickr.

### Algorithm

Given a list of images with known `(width, height)` and a target row height `H`:

1. For each image compute its *scaled width* at height `H`: `w_i = H × (W_i / H_i)`.
2. Greedily pack images into rows. Keep a running total of scaled widths (plus gaps).
   When adding the next image would exceed the container width `C`, close the current row.
3. For a closed row containing images `1..n`, compute the actual row height:
   `h_row = C / Σ(W_i / H_i)` — i.e. scale all images uniformly so their total
   width equals `C` exactly. Each image's display width becomes `h_row × (W_i / H_i)`.

Because all images in a row are scaled by the same factor, aspect ratios are preserved
exactly. `object-fit: cover` is not needed; each image fills its container perfectly.
Cropping is zero.

### Row-height variance and clamping

Each row closes when `Σ w_i ≥ C`. The overshoot is at most one image width. For typical
photo mixes the actual row height stays within ±30 % of the target `H`, but pathological
mixes can blow this out: e.g. one portrait (W/H ≈ 0.67) plus one landscape (W/H ≈ 1.5)
on a wide row produces a row much taller than `H`. To bound this, clamp row height to
`max_h = 1.5 × H`. When the clamp activates, the row is left under-filled rather than
stretched — visually identical to the current ragged-edge behaviour, but only in the
worst cases.

### Last-row handling

The last (partial) row needs an explicit policy — leaving it raw produces ugly large
gaps when only one or two images trail. Decision: if the last row contains fewer than
3 items *and* its scaled-width fill ratio is below 50 %, merge it with the previous
row and re-justify; otherwise leave it at height `H` unjustified. This keeps the common
case clean without ever stretching a single trailing photo to absurd width.

### Videos

Videos currently render full-width (`gallery-item--video`, `style.css:368`). They keep
that behaviour: a video closes the current row, occupies its own full-width row, and the
next image starts a fresh row. Videos do not participate in the packing algorithm.

### EXIF hover overlay

The `.gallery-exif` overlay (`style.css:327`) relies on `.gallery-item` being
`position: relative` with bounded dimensions. The new layout still gives each item
explicit width and height, so the overlay continues to work unchanged.

### Relationship to panoramic detection

The current panoramic threshold (`width > 2 × height`, `gallery-item--panoramic`,
`style.css:363`) becomes redundant — very wide images naturally form a row by themselves
because their scaled width already fills or nearly fills the container. The class can
be removed.

## Implementation: CSS-only via flex-grow

Compute row groupings server-side in `theme/mod.rs` when building `PhotoGroupCtx`.
Replace the flat `Vec<MediaCtx>` with `Vec<RowCtx>` where each row holds its images
with their aspect ratios. Templates render:

```html
<div class="gallery-row">
  <a class="gallery-item" style="flex-grow: 1.5; aspect-ratio: 1.5">…</a>
  <a class="gallery-item" style="flex-grow: 0.67; aspect-ratio: 0.67">…</a>
</div>
```

CSS:

```css
.gallery-row { display: flex; gap: 0.375rem; }
.gallery-item { flex-basis: 0; }  /* grow proportionally to aspect ratio */
```

`flex-grow` proportional to aspect ratio plus `aspect-ratio` on each item gives
justified rows that reflow on viewport resize for free — no JavaScript, no fixed-width
assumption. The server only decides *which images share a row*; the browser handles
the actual scaling.

This sidesteps the main weakness of a pure server-side pixel layout (which would be
computed for one assumed container width and fail on other viewports) while keeping
zero JavaScript.

### What the server needs

- Image `(width, height)` on every `MediaItem` — already read at load time.
- A target row height `H` and an assumed container width `C` for the *grouping*
  decision only (not for final pixel sizes). Reasonable defaults: `H = 280 px`,
  `C = 1160 px`. Slight viewport-vs-assumed-`C` mismatches just shift fill ratios
  marginally; rows still justify correctly because flex handles the final scaling.

### Open question: row regrouping on narrow viewports

On a phone (≈ 380 px wide) the desktop row groupings will pack too many images per
row, making each one tiny. Mitigations:
- A second media-query breakpoint that drops to 1–2 items per row via CSS only.
- Or a small JS pass that re-groups on resize (kept as a fallback if CSS proves
  insufficient).

Start CSS-only and add JS only if the mobile layout looks bad in practice.
