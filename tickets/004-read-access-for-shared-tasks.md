# 004: Read access for shared tasks

## Goal

Allow users with a `read` or `read_write` share to read a task, and include
shared tasks in the listing endpoints.

## Background

Currently non-owners can never read a task:

- `GET /task_by_id` returns 401 unless the caller owns the task.
- `GET /all_tasks` and `GET /incomplete_tasks` only return tasks the user owns
  (see `get_all_tasks_for_user` / `get_incomplete_tasks_for_user` in
  `src/core.rs`).

After tickets 001–003, share rows exist and can be looked up. Design decision:
listing endpoints merge owned and shared tasks. Consumers can distinguish
tasks by checking `owner` vs. the current user's id.

## In scope

- `GET /task_by_id`: allow access when the caller has `ReadOnly` or
  `ReadWrite` permission via `get_task_permission` (ticket 002) in addition to
  ownership. No-access users — including when the task does not exist — still
  get the same 401 (existence-oracle rule, see ticket 003); do not leak a
  404-vs-401 distinction.
- `get_all_tasks_for_user` / `get_incomplete_tasks_for_user` in `src/core.rs`:
  return tasks the user owns **or** that are shared with them (join
  `task_shares` on `task_shares.task_id = tasks.id AND task_shares.user_id =
  user.id`, or a union of two queries). Owned tasks only appear once even if a
  stale share row also exists.
- Both `/all_tasks` and `/incomplete_tasks` handlers inherit this via the core
  functions above — no handler changes needed if the core functions are
  updated.

## Out of scope

- Write access (ticket 005).
- Changing response shapes or adding an explicit "shared_by" marker. Instead
  of embedding usernames in task payloads, ticket 007 adds a `GET /user_by_id`
  endpoint so clients can resolve the `owner` id on demand.

## Acceptance criteria

- Integration tests: a user with a `read` share can `GET /task_by_id` and sees
  the task in `/all_tasks`; with no share they still get 401 and no listing
  entry; the owner's listings are unchanged and never duplicate.
- `cargo test` and `cargo clippy` pass.
