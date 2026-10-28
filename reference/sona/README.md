# Sona reference engine (verbatim copy)

This folder is a verbatim copy of the top-level `*.js` files of the Sona v11
sibling checkout's `dist/` folder.

- **Source checkout:** `/root/nmc/worktrees/79/sona` (read-only sibling of the haa checkout)
- **Source commit:** `5849617e0ef8798ca4d423902f13afaa2e7ad94f`
- **Copy date:** 2026-10-28

Every file here is copied byte for byte from Sona's `dist/`. That includes
`dsp.js`, `audio-worklet.js`, `catalog.js`, the node modules,
`modulation-*.js`, `presets.js`, `*-preset.js` and `*-studies.js`.

## Rule

These files are **never edited**. They are refreshed only by copying them
again from the Sona checkout's `dist/` (and updating the commit above).

## Use

`tools/listen` loads them at runtime by URL (`/reference/sona/...`) as the
A-side reference engine. Nothing in `crates/` or `packages/` imports them;
they are served as static files by the listen server.
