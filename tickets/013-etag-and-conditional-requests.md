# 013: Add `ETag`, `Cache-Control`, and conditional requests

**Phase:** media types, headers, and status codes (after the redesign)
**Depends on:** 001 (same response code path), 006
**Files:** `src/web_api.rs`, `src/core.rs`, `migrations/`, `src/schema.rs`

## Goal

Let callers revalidate cached representations cheaply, and stop concurrent
editors of the same task from silently overwriting each other.

## Why this comes after the redesign

The background below describes the current `GET /task_by_id` and
`PATCH /task_by_id`. The work is on the task resource either way, and it needs
a schema migration plus a decision about whether `If-Match` is required — both
easier to decide once the task URI is settled. The migration itself is
independent of the URIs and could land earlier if wanted.

## Background

REST's cache constraint (Fielding's constraint 5) expects cacheable
representations to carry validators. There are none today: a `grep` for `ETag`,
`Last-Modified`, `Cache-Control`, `Vary`, `If-Match`, and `If-None-Match`
across `src/` returns nothing.

Two consequences:

- **No revalidation.** `GET /task_by_id` cannot be revalidated, so a client
  must refetch the whole representation to know whether it changed. There is
  no `304 Not Modified` path.
- **Last write wins.** `PATCH /task_by_id` is unprotected against concurrent
  edits. A `read_write` sharee and the owner can both update the same task, and
  whichever request lands last silently discards the other's change. The
  sharing feature added in tickets 001–013 of the previous effort made this
  two-writer case reachable; nothing was added to detect the conflict.

## In scope

- **A version or timestamp to validate against.** `tasks` has no `updated_at`
  or version column. Add one via a Diesel migration under `migrations/` (both
  `up.sql` and `down.sql`, matching the existing migration layout) and
  regenerate `src/schema.rs` with `diesel print-schema`.
- **`ETag` on `GET /tasks/{id}`**, derived from the task's version. A strong
  validator (`"<id>-<version>"`) is simplest and honest; a weak validator is
  acceptable if a content hash is used instead.
- **`304 Not Modified`** when `If-None-Match` matches, with no body.
- **`If-Match` on `PATCH /tasks/{id}`**, returning `412 Precondition Failed`
  when it does not match. RFC 9110 §13.1.1 requires `If-Match` to be honored on
  state-changing requests when supplied, so it must not be ignored when
  present. Whether it is *required* is a free choice here — there are no clients
  to grandfather. `If-Match: *` means "the resource must exist".
  - **Decision**: If-Match is required.
- **Cache directives on authenticated GETs.** Responses are per-user and
  per-auth-key, so they must never be stored in a shared cache: at minimum
  `Cache-Control: private, no-store` and `Vary: Authorization`. Decide whether
  any authenticated representation is safe to cache privately at all; if not,
  say so in the README rather than leaving it unspecified.
- Keep the ordering sane: an `If-Match` mismatch (412) should be reported
  before, not after, a permission failure (403 from ticket 003).

## Out of scope

- `Last-Modified` and `If-Modified-Since` as well as ETags. ETag alone is
  sufficient and avoids clock-skew questions; add the date form only if a
  client needs it.
- ETag on the collection endpoint (`GET /tasks`). It is per-user, filtered, and
  cheap to refetch. Ticket 007 has already collapsed it to one URI by the time
  this lands.
- Optimistic concurrency for shares.

## Acceptance criteria

- [ ] `tasks` has a version column, added by a reversible Diesel migration,
      with `src/schema.rs` regenerated.
- [ ] `GET /tasks/{id}` returns a strong `ETag` that changes when the task
      changes and stays the same when it does not.
- [ ] A `GET` with a matching `If-None-Match` returns `304` with no body.
- [ ] A `PATCH` with a stale `If-Match` returns `412 Precondition Failed` and
      does **not** modify the task.
- [ ] Whether `If-Match` is required is decided and documented; a `PATCH`
      without it either works or answers `428 Precondition Required`.
- [ ] A `PATCH` with a matching `If-Match` succeeds.
- [ ] Authenticated GET responses carry `Cache-Control: private, no-store` and
      `Vary: Authorization`.
- [ ] A test covers the two-writer race: two `read_write` grantees fetch the
      same task, one PATCHes with `If-Match`, the other's stale `If-Match` is
      rejected.
- [ ] `README.md` documents the validators and the conditional-request
      behavior for `/tasks/{id}`.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

The migration runs against the test database, so `scripts/run_test_database.sh`
plus `DATABASE_URL=postgres://postgres:test-password@localhost/todo` is needed
before the suite will pass. Diesel migrations in this repo are reversible —
keep it that way so a rollback is possible.

`no-store` is the honest default here: task listings are small, and the risk of
serving one user's tasks to another outweighs the bandwidth saved.
