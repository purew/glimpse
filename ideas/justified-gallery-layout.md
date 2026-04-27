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

### Algorithm (server-side or JS)

Given a list of images with known `(width, height)` and a target row height `H`:

1. For each image compute its *scaled width* at height `H`: `w_i = H × (W_i / H_i)`.
2. Greedily pack images into rows. Keep a running total of scaled widths (plus gaps).
   When adding the next image would exceed the container width `C`, close the current row.
3. For a closed row containing images `1..n`, compute the actual row height:
   `h_row = C / Σ(W_i / H_i)` — i.e. scale all images uniformly so their total
   width equals `C` exactly. Each image's display width becomes `h_row × (W_i / H_i)`.
4. The last (partial) row is left unjustified — images stay at height `H` without
   stretching, leaving a gap on the right.

Because all images in a row are scaled by the same factor, aspect ratios are preserved
exactly. `object-fit: cover` is not needed; each image fills its container perfectly.
Cropping is zero.

### Why row height stays close to `H`

Each row closes when `Σ w_i ≥ C`. The overshoot is at most one image width. For typical
photos (width ≤ 2× H), the actual row height stays within roughly ±30 % of the target.
Choosing a larger target `H` tightens the variance.

### Implementation options

**Server-side (Rust + template)**

Compute row groupings in `theme/mod.rs` when building `PhotoGroupCtx`. Replace the flat
`Vec<MediaCtx>` with `Vec<RowCtx>` where each `RowCtx` holds the images and their computed
pixel widths/heights. Templates render `<div class="gallery-row">` wrappers with explicit
`width` and `height` inline styles per image.

Requires image dimensions on every `MediaItem`, which are already read at load time.
No JavaScript; layout is baked into the HTML. Reflow is not needed because dimensions
are absolute pixels — but this means the layout is computed for a fixed assumed container
width (e.g. 1160 px) and won't reflow on resize.

**Client-side (JavaScript)**

Pass `data-w` and `data-h` attributes on each `<img>`. On load (and on `resize`), run the
packing algorithm in JS and apply `width`/`height` styles. Handles any container width and
reflows correctly on window resize.

Adds a small JS dependency (~30 lines of vanilla JS). Layout shifts slightly on first load
unless image dimensions are pre-set in HTML.

**Hybrid**

Server bakes the row groupings for the expected desktop width; a small JS pass adjusts
widths on resize without re-grouping rows.

### Relationship to panoramic detection

The current panoramic threshold (`width > 2 × height`) would become unnecessary — very
wide images naturally form a row by themselves because their scaled width already fills
or nearly fills the container.
