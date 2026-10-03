# 003: Share management endpoints

## Goal

Let task owners grant, revoke, and review access to their tasks through new
HTTP endpoints.

## Background

After tickets 001–002, `task_shares` exists and permissions can be resolved,
but there is no way to create or remove share rows through the API. Only
existing patterns to follow: every endpoint authenticates via
`auth_key_to_user`, uses a tracing span with a request id, and returns JSON
with `{"request_id": ...}`.

## In scope

Add (all JSON in/out, following existing payload-struct patterns in
`src/web_api.rs`):

- `POST /share_task` — input: `auth_key`, `id`, `username`, `permission`
  (`"read"` | `"read_write"`). The caller must own the task. The target
  `username` must exist. If the user already has a share row for the task,
  replace its permission (upsert semantics per ticket 001's unique row).
  Returns 200 with `{"request_id": ...}`.
  **Existence-oracle rule:** return the *same* error (401 `Unauthorized`) and
  the *same* body shape when the task does not exist as when it exists but the
  caller doesn't own it. A 404 for a missing task but 401 for an owned-by-
  someone-else task would let a caller enumerate which task IDs exist.
  Unknown target username may still return 404 (that leaks usernames, which is
  acceptable — sharing requires naming the user).
- `DELETE /unshare_task` — input: `auth_key`, `id`, `username`. Caller must
  own the task. Deletes the share row. Apply the same existence-oracle rule
  here: missing task and not-your-task are both 401. A missing share row on a
  task you own may return 404 (no enumeration risk for the owner's own task).
- `GET /task_shares` — input: `auth_key`, `id`. Caller must own the task.
  Returns the list of shares for the task: `[{"username": ..., "permission":
  ...}]`. Same existence-oracle rule for missing-vs-not-your-task.

Core functions in `src/core.rs` to back these (e.g. `add_share_for_task`,
`remove_share`, `list_shares_for_task`), plus accessor on `TaskShare`.

Register all three endpoints in the `build_app!` macro in `src/lib.rs`.

## Out of scope

- Letting shared users read/write via these permissions (tickets 004, 005).
- Notification of the recipient user.

## Acceptance criteria

- New integration tests in `mod tests` in `src/web_api.rs`: owner can share
  and see the share listed; re-sharing changes the permission; non-owner
  caller gets 401; unsharing removes the row; unknown target username gives
  404; and a missing task returns the same 401 as a not-owned task (no
  existence oracle).
- `cargo test` and `cargo clippy` pass.
- Manual request files can be added under `http_requests/` (full list in ticket 006).
