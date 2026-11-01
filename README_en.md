# Kblog

A personal blog engine with a built-in admin backend: Rust + Axum + MiniJinja + SQLite.

[English](README.md) · [中文说明](README_zh.md)

## What it is

A single-process blog server. The public side is server-rendered, static-looking pages (post archives, notes, about); the admin lives in the same process and handles site settings, credentials, and post CRUD/publishing. No frontend build step, no JS framework, and the database is one SQLite file.

## Features

- **Server-side rendering** with MiniJinja templates and per-page CSS; light/dark follows `prefers-color-scheme`
- **Built-in admin**: login, dashboard, site settings, post list (20 per page) / create / edit / publish / unpublish / delete
- **Drafts are invisible** — a draft returns 404 on both the archive and the detail page instead of shipping a half-baked "preview while logged in" flow
- **Markdown body**: tables, footnotes, strikethrough, task lists (`pulldown-cmark`)
- **Generated slugs**: lowercased, non-alphanumerics folded to `-`; when a title yields nothing usable it falls back to `{date}-{8 hex chars}`
- **Deployable with no external services**: one binary plus the SQLite file and the `templates/` / `assets/` directories
- **Response compression**: gzip / brotli (`tower-http` CompressionLayer)

## Quick start

Requires Rust 1.85 or newer (`Cargo.toml` uses `edition = "2024"`).

```bash
git clone https://github.com/korbinZhang/kblog.git
cd kblog
cargo run
```

On first start, a random 12-character initial password is generated and printed once:

```
INFO kblog::admin: 已生成后台初始密码: <12位随机码>
INFO kblog:        后台登录页: http://127.0.0.1:3000/admin/login
```

Open <http://127.0.0.1:3000> for the blog and `/admin/login` for the admin. **Change the password in Site Settings right after logging in** — rotating it invalidates every existing session.

For a release build use `cargo build --release`; the binary lands in `target/release/kblog`.

## Configuration

Everything comes from environment variables, loaded from an optional `.env` in the working directory via `dotenvy`. Start from the shipped template:

```bash
cp .env.example .env
```

Unset keys fall back to the defaults hardcoded in `src/config.rs`.

| Variable | Default | Purpose |
|---|---|---|
| `WEB_ADDR` | `127.0.0.1` | Bind address |
| `WEB_PORT` | `3000` | Bind port |
| `TEMPLATES_PATH` | `templates` | Template directory |
| `ASSETS_PATH` | `assets` | Static directory, served by the fallback |
| `DATABASE_PATH` | `data/data.db` | SQLite file, created on demand |
| `ADMIN_PATH` | `admin` | Admin prefix, i.e. `/{ADMIN_PATH}/login` |

`ADMIN_PATH` must be non-empty, at most 64 bytes, limited to ASCII alphanumerics plus `-` and `_`, and must not collide with the public paths `blog` / `thinking` / `about`. A rejected value logs a `WARN` and falls back to `admin`. Changing it requires a restart; the old URL starts 404ing immediately.

Site name, description, URL and the about page live in the `settings` table and are edited from the admin, not from the environment. Password and sessions are in the database as well — deliberately kept out of `Config`, which `Config::init()` pretty-prints in full.

## Routes

Public:

| Path | Purpose |
|---|---|
| `/`, `/blog`, `/blog/{year}` | Yearly post archive; the bare form uses the latest year with posts |
| `/blog/{year}/{slug}` | Post detail |
| `/thinking`, `/thinking/{year}` | Notes |
| `/about` | About page |
| `/{ADMIN_PATH}/login` | Admin login |

Admin (under the `/{ADMIN_PATH}` prefix):

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET / POST | `/login` | public | Sign in |
| POST | `/logout` | public | Sign out |
| GET | `/` | required | Dashboard: totals, published, drafts, 5 most recent |
| GET / POST | `/settings` | required | Site information |
| POST | `/password` | required | Change password |
| GET | `/posts` | required | Post list, `?page=` pagination |
| GET / POST | `/posts/new` | required | Create |
| GET / POST | `/posts/{id}` | required | Edit; the submitted button decides the published state |
| POST | `/posts/{id}/publish` · `/unpublish` | required | Publish / revert to draft |
| POST | `/posts/{id}/delete` | required | Delete, guarded only by a browser `confirm()` |

Unauthenticated requests to any protected path return **404**, not a 302 to the login page, so the admin is indistinguishable from a path that never existed.

## Layout

```
src/
  main.rs        Router, public handlers, AppState
  config.rs      Env parsing and ADMIN_PATH validation
  db.rs          SQLite schema and every query
  admin.rs       Admin routes, sessions, argon2 password handling
  markdown.rs    pulldown-cmark setup
templates/       base / blog / thinkings / post / about, plus 6 admin templates
assets/          base / archive / blog / thinkings / post / admin CSS plus favicons
data/            SQLite file (gitignored)
```

## Data model

`src/db.rs::create_tables()` runs raw DDL — there is no migration framework:

```
settings(key PK, value)              site_name / site_description / site_url / author / about_content / admin_password
posts(id PK, slug UK, title, description, content, published, datetime, updated_at)
thinkings(id PK, datetime, content)  public notes
admin_sessions(token PK, created_at, expires_at)
```

Input limits: 200 characters for a title, 500 for a description, 200,000 for the body. A failed validation re-renders the form with an error banner instead of discarding what you typed.

## Security

Handled: HTML auto-escaping is explicitly enabled (MiniJinja's default callback ignores the `.jinja` extension, so without this a title saved in the admin would be stored XSS); argon2id hashes with constant-time verification; server-side session tokens expire after 7 days with `HttpOnly`, `SameSite=Lax` and `Path=/{ADMIN_PATH}`, revoked on logout and password change; every mutation is POST; listings use `LIMIT/OFFSET` and forms are length-capped.

Accepted boundaries — they assume a single trusted admin: raw HTML in the body is not sanitized (add an allowlist scrubber such as `ammonia` before allowing co-authors or guest posts); no CSRF tokens, relying on "all writes are POST + SameSite=Lax"; the cookie has no `Secure` flag because the app runs on plain http locally, and must set it under HTTPS; session tokens are stored in plaintext; deletion is irreversible; the initial password is logged once.

## Known limitations

- Notes have a public page but no admin UI; they must be inserted straight into the database
- There is no draft preview — seeing a draft rendered means publishing it
- No RSS, sitemap, comments, or tag/category pages
- No automated tests; the UI is verified by hand

Please report security issues privately rather than opening a public issue.

## License

[MIT](LICENSE) © 2026 korbin
