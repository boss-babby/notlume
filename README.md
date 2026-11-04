# Notes — a Notion-style workspace in Rust

Native Windows app + WebAssembly web app from **one Rust codebase** (egui, no WebView / Electron / DOM UI),
a Rust API server, and a website that handles accounts, hosts the web build and distributes installers.

> **Read this first — honest status.** The authoring environment had no Rust toolchain, so the Rust crates were
> written carefully but **never compiled**. The website (`src/`) *was* built and exercised end-to-end.
> Expect a short `cargo check` fix-up pass on the Rust side (egui API details). See
> [docs/ROADMAP.md](docs/ROADMAP.md) for the per-feature status and
> [docs/adr/0001-architecture.md](docs/adr/0001-architecture.md) for decisions and deviations
> (egui pinned to 0.31.1; website in Next.js instead of Leptos; `localStorage` instead of IndexedDB;
> LWW-per-block sync with a documented path to `yrs`).

## Architecture

```
crates/
  core/        nw-core   block tree, rich text (marks), markdown, HTML/JSON/PDF export, undo/redo,
                         fuzzy + full-text search, databases (filter/sort/group), LWW block-CRDT sync, benches
  ui/          nw-ui     egui: theme + animation presets, block editor, sidebar, palette, databases, app shell
  app-native/  notes     desktop entry point: SQLite persistence, file dialogs, wgpu
  app-web/     app-web   wasm entry point (trunk): localStorage persistence, downloads/imports, glow
  api-types/   nw-api-types  request/response structs shared by server and apps
  server/      server    axum + sqlx/PostgreSQL: argon2 auth, sessions, verify/reset, sync, serves /app
src/                     website (Next.js): landing, auth, dashboard, downloads, /app, /workspace, same API contract
docker/  .github/workflows/  scripts/  docs/
```

Data flow: `nw-ui::NotionApp` talks only to the `Persistence` trait (SQLite / localStorage) and `SyncClient`
(`ehttp`, works on native and wasm). Pages are JSON (`nw_core::Page`); the server stores them opaquely and
clients merge with `nw_core::sync::merge_pages`.

## Prerequisites

* Rust stable (`rustup`), `wasm32-unknown-unknown` target, [`trunk`](https://trunkrs.dev) for the web build
* Node 22+ and PostgreSQL 14+ for the website / server
* Windows packaging: `cargo install cargo-packager --locked` (WiX for MSI, NSIS for the setup EXE)
* Linux builds: `libxkbcommon-dev libwayland-dev libx11-dev`

## Run

```bash
cp .env.example .env          # edit DATABASE_URL etc.

# 1. native desktop app (SQLite in %APPDATA%\Notes\workspace.db, override with NW_DB)
cargo run -p app-native --release

# 2. web app (WASM) — dev server on :8080
cd crates/app-web && trunk serve
#    production build (output: crates/app-web/dist, served by the website / server at /app)
cd crates/app-web && trunk build --release

# 3. Rust API server (runs migrations, optionally serves the wasm build at /app)
cargo run -p server

# 4. website
npm install && npx drizzle-kit push && npm run dev      # http://localhost:3000
```

Smaller wasm: `CARGO_PROFILE_RELEASE_OPT_LEVEL=s trunk build --release` (wasm-opt `-Oz` is applied by trunk).
Bundled Inter font: `scripts/fetch-fonts.sh` then `cargo run -p app-native --release --features nw-ui/bundled-fonts`.

### How the site, web app and desktop app connect

* The website's **Open Web App** button goes to `/app`. If `crates/app-web/dist` exists (`WASM_DIST`) it is served
  there with the same login cookie; otherwise `/app` redirects to `/workspace`, a TypeScript preview editor that
  reads/writes the same page JSON.
* The web app links back to the website and to `/download` (sidebar footer, top bar, palette, `index.html`).
* The desktop app signs in from *Settings → Sync* (bearer token) and syncs to the same account.

## Test, lint, benchmark

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                                   # core, ui, native store, server unit tests
TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/app_db cargo test -p server   # integration
cargo bench -p nw-core                                   # editor (rich text), search, sync
npx tsc --noEmit && npm run build                        # website
```

## Package for Windows

```powershell
./scripts/package-windows.ps1      # dist/*.msi, *-setup.exe, *-portable.zip, SHA256SUMS.txt
./scripts/sign-windows.ps1         # optional: needs WINDOWS_CERT_BASE64 / WINDOWS_CERT_PASSWORD
```

Copy the artefacts into `public/downloads/`; `/download` then shows size and SHA-256 for each (it never invents
checksums — missing files are shown as "not published yet"). CI: `.github/workflows/ci.yml` (fmt, clippy, tests
with Postgres, wasm build + gzip size, website) and `release.yml` (tag → MSI/EXE/ZIP GitHub release).

## Deploy

`docker compose -f docker/docker-compose.yml --env-file .env up --build` — PostgreSQL, Rust server (+ wasm app),
website and Caddy with automatic HTTPS. Details and security checklist: [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).

## Using the app

| | |
|---|---|
| `/` | block menu · `@` link a page · markdown: `# ` `- ` `[] ` `> ` ``` ``` `---` |
| `Ctrl+K` | command palette (pages, full text, commands) |
| `Ctrl+B / I / E / Shift+S / H` | bold / italic / code / strike / highlight selection |
| `Ctrl+Z`, `Ctrl+Shift+Z` | undo / redo (typing bursts coalesce) |
| `Ctrl+\` | toggle sidebar |
| drag `⠿` | reorder blocks (ghost preview, spring-snapped drop line); drag pages in the sidebar to nest / reorder |

Animations all go through `crates/ui/src/anim.rs` (`ease_out_cubic`, `ease_in_out_cubic`, `ease_out_back`,
`spring`, `Spring`); *Reduce motion* (Settings, or the OS preference on web) sets egui's animation time to 0.

## License

MIT
