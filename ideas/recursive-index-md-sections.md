# Recursive index.md sections

Support additional `index.md` files placed in subfolders beneath a post root. Each such file would add a named section to the parent blog post, with its own Markdown body and the photos from that subfolder rendered beneath it.

## Motivation

Currently a post is a single flat narrative (`index.md` at root) with photos grouped by subfolder name. Recursive `index.md` files would let each day or location within a trip carry its own prose without cramming everything into the root file.

## Expected behaviour

```
posts/
  2025-03-18 Hawaii/
    index.md              # post root — title, date, access, cover
    2025-03-18 Travel day/
      index.md          # section — own Markdown body
      *.jpg
    2025-03-19 Manoa Falls/
      index.md          # section — own Markdown body
      *.jpg
```

- Each subfolder `index.md` renders as a section within the parent post page.
- Section order follows subfolder sort order (date-prefixed names sort naturally).
- Frontmatter for section files: `title` (optional, falls back to folder name).
- Access control is inherited from the post root; section files do not define their own `access`.

## Affected modules

- `content` — scan for nested `index.md` during post load; attach parsed body to `PhotoGroup`.
- `theme` — render section title + body above each `PhotoGroupCtx` in `post.html`.
