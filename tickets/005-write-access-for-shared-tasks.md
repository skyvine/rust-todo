# 005: Write access for shared tasks

## Goal

Allow users with a `read_write` share to modify a task's fields; users with
only `read` (or no share) remain unable to.

## Background

`update_task` in `src/core.rs` currently requires ownership:

```rust
if task.owner != owner.id {
    return Err(ApplicationError::Unauthorized);
}
```

The permission helper from ticket 002 and the read-access changes from ticket
004 are in place. This ticket extends the same `get_task_permission` check to
mutations, with stricter requirements (`ReadWrite` or `Owner`).

## In scope

- Change `update_task` (in `src/core.rs`) to accept the caller and allow the
  update when their permission is `Owner` or `ReadWrite`; reject
  `ReadOnly`/none with `ApplicationError::Unauthorized`. Rename the first
  parameter if it is no longer always the owner (e.g. `actor`/`caller`).
  Apply the existence-oracle rule: missing task and insufficient permission
  produce the same 401.
- Audit any other mutation paths (there are no delete-task endpoints today);
  `add_task` always creates a task owned by the caller and is unchanged.
- Shares are still owner-manageable only (ticket 003): a `ReadWrite` grantee
  must not be able to add/remove shares via `/share_task`, `/unshare_task`,
  or `/task_shares`. Verify this holds; add a test if not covered.

## Out of scope

- Deleting tasks (no endpoint exists).
- Transferring task ownership.

## Acceptance criteria

- Integration tests: `read_write` sharee can update title/description/
  completed via `POST /task_by_id`; `read`-only sharee gets 401; unrelated user
  gets 401; owner behavior unchanged.
- A `read_write` sharee cannot call `/share_task` etc. successfully (401/403).
- `cargo test` and `cargo clippy` pass.
