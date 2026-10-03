# 006: Docs and manual test examples

## Goal

Document the sharing feature for users and keep the manual-testing assets in
`http_requests/` current.

## Background

`README.md` documents every endpoint (input/output keys) and there are
example raw request files in `http_requests/`. These must reflect the new
sharing endpoints and the changed access rules for existing endpoints.

## In scope

- Update `README.md`:
  - Document `POST /share_task`, `DELETE /unshare_task`, `GET /task_shares`
    (input keys and responses), placing them in the right sections alphabetically/by method like the rest.
  - Update the descriptions of `GET /task_by_id`, `GET /all_tasks`,
    `GET /incomplete_tasks`, and `POST /task_by_id` to state that shared
    access (read or read-write) is honored.
- Add example files under `http_requests/`, e.g. `post_share_task.http`,
  `delete_unshare_task.http`, `get_task_shares.http`, following the existing
  file format.
- Update `tickets/README.md` if scope or decisions changed during
  implementation.

## Acceptance criteria

- Every endpoint implemented in tickets 003–005 appears in `README.md` with
  accurate input/output documentation.
- Each new endpoint has a corresponding example in `http_requests/`.
- No code changes; `cargo test` still passes.
