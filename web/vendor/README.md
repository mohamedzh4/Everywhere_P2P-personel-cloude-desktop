# Vendored libraries

Copied unmodified from npm so the page never loads code from a CDN at runtime.

| Folder | Package | Version | Licence |
|---|---|---|---|
| `noble-curves/` | `@noble/curves` | 2.4.0 | MIT (see `noble-curves/LICENSE`) |
| `noble-hashes/` | `@noble/hashes` | 2.4.0 | MIT (see `noble-hashes/LICENSE`) |

Only the `.js` files are kept (no `src/`, typings or source maps). The import map in `index.html`
points `@noble/curves/` and `@noble/hashes/` here.

To update: `npm pack @noble/curves@X @noble/hashes@X`, copy the same files, re-run `node test/run.mjs`.
