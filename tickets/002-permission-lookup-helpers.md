# 002: Permission lookup helpers

## Goal

Provide a single core function that answers: "what can this user do with this
task?" — and migrate existing ownership checks to use it.

## Background

Authorization today is scattered and owner-only:

- `get_task_by_id` handler in `src/web_api.rs` compares
  `user.ref_id() == task.owner_id()`.
- `update_task` in `src/core.rs` fetches the task and returns
  `ApplicationError::Unauthorized` unless `task.owner == owner.id`.
- `get_all_tasks_for_user` / `get_incomplete_tasks_for_user` filter by
  `owner.eq(user.id)` only.

After ticket 001, the `task_shares` table exists. This ticket adds the logic
to interpret it without yet changing endpoint behavior for shared tasks.

## In scope

- Add a `Permission` enum (e.g. `Owner`, `ReadWrite`, `ReadOnly`, `None` or
  `Option`-based equivalent) in `src/core.rs`.
- Add `get_task_permission(user: &User, task_id: &i32, connection: &mut PgConnection) -> Result<Permission, ApplicationError>`
  which:
  - returns `Permission::Owner` if the user owns the task,
  - otherwise looks up `task_shares` for (task_id, user_id) and maps
    `read` → `ReadOnly`, `read_write` → `ReadWrite`,
  - returns an "no access" variant (or `ApplicationError::Unauthorized`) when
    there is no row.
- Refactor the existing owner checks in `update_task` and the
  `get_task_by_id` handler to go through this helper, preserving current
  observable behavior for owners and non-owners with no share row.

## Out of scope

- Actually granting non-owners access anywhere (tickets 004, 005).
- Creating/deleting share rows (ticket 003).

## Acceptance criteria

- `cargo test` passes; existing tests for owner-only behavior are unchanged.
- New unit tests (in `src/core.rs` or an integration test following the
  patterns in `mod tests`) cover: owner, read-write share, read-only share,
  and no relationship.
- No new endpoint behavior yet — shared users still get 401/403 on
  task endpoints.
