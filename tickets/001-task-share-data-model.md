# 001: Task share data model

## Goal

Introduce the database representation for task shares: a row linking a task to
a user plus the level of access granted (`read` or `read_write`).

## Background

Today, only a task's owner can see or modify a task. Ownership is stored in
`tasks.owner` (see `src/schema.rs` and the `Task` struct in `src/core.rs`).
Sharing requires a new table; nothing else about the existing schema should
change.

## In scope

- Add a new Diesel migration under `migrations/` (follow the naming pattern of
  `2025-08-11-213735_create_tasks_table`) that creates a `task_shares` table:

  ```sql
  CREATE TABLE task_shares (
      id SERIAL PRIMARY KEY,
      task_id integer NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
      user_id integer NOT NULL REFERENCES users(id) ON DELETE CASCADE,
      permission text NOT NULL CHECK (permission IN ('read', 'read_write')),
      UNIQUE (task_id, user_id)
  );
  ```

  The `UNIQUE (task_id, user_id)` constraint means a user has at most one share
  row per task; updating access means replacing that row. `ON DELETE CASCADE`
  ensures deleting a task or user removes its share rows. The matching
  `down.sql` must drop the table.

- Regenerate/update `src/schema.rs` for the new table, including
  `diesel::joinable!(task_shares -> tasks ...)` points if you use joins.

- Add to `src/core.rs`:
  - A `SharePermission` enum with variants `Read` and `ReadWrite`, usable by
    core code and constructed from the stored text value (parse and return
    `ApplicationError::InvalidData` on unknown values).
  - A `TaskShare` struct that is `Queryable`/`Selectable` against `task_shares`,
    with accessor methods, matching the style of `Task`/`AuthKey`.

## Out of scope

- Any endpoints or functions that read/write shares (tickets 002–005).
- Surfacing share info in existing responses.

## Acceptance criteria

- `diesel migration run` (and `diesel migration revert`) succeed against a
  scratch database.
- `cargo build` succeeds with no new warnings; `src/schema.rs` contains a
  `task_shares` table.
- A small unit or integration test in `src/core.rs`'s vicinity (or a manual
  `psql` session) confirms a `task_shares` row can be inserted and selected,
  and that the unique constraint rejects duplicate (task_id, user_id) pairs.
- No existing endpoint behavior changes.
