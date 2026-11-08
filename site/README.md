# Niles site

The landing page: a man page in a terminal, built with Astro into static files.
Requires Node 24 and pnpm 10.8.1 (pinned in package.json).

```sh
cd site
pnpm install --frozen-lockfile
pnpm dev
```

`pnpm typecheck`, `pnpm test` and `pnpm build` run in CI; `dist/` is the whole site, served at a
site root. Colours follow the PWA's tokens in `web/src/style.css`.

- `src/demo/` is one staged session with two views: the home view (`model.ts`, `view.ts`) and the
  remote PWA on a phone (`phone.ts`), sharing state through `store.ts`, so a message sent from
  the phone shows in the terminal. Nothing in it talks to a real Niles.
- `src/repo/` reads the repository at build time: the logo as text art from
  `assets/niles-lockup-amber.svg`, the TODO list from `ROADMAP.md` and the version from
  `Cargo.toml`. A change to their shape fails the build.

Vercel builds and serves it from git (project `niles`, root directory `site`, production branch
`main`) at https://niles-xi.vercel.app, which `site` in astro.config.mjs names for the canonical
link and the share image. The build reads files outside `site/`, so the project keeps "Include
source files outside of the Root Directory" on. `public/og.png` is a 1200x630 capture of the
page's first screen; take it again when the first screen changes.
