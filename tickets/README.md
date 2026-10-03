# Feature: Task Sharing — Ticket Breakdown

This feature lets a task owner share a task with other users, granting either
read-only or read-write access. The work is split into the tickets below so
each piece can be implemented, reviewed, and tested independently. Each ticket
file is self-contained: it includes the background, the relevant files, the
in-scope work, and acceptance criteria.

## Tickets (in dependency order)

1. [001-task-share-data-model.md](001-task-share-data-model.md) — Database table,
   Diesel schema, and Rust types for shares.
2. [002-permission-lookup-helpers.md](002-permission-lookup-helpers.md) — Core
   helper that resolves a user's effective permission on a task, plus tests.
3. [003-share-management-endpoints.md](003-share-management-endpoints.md) —
   Endpoints to add/remove/list shares (owner only).
4. [004-read-access-for-shared-tasks.md](004-read-access-for-shared-tasks.md) —
   `/task_by_id`, `/all_tasks`, `/incomplete_tasks` honor read permissions on
   shared tasks.
5. [005-write-access-for-shared-tasks.md](005-write-access-for-shared-tasks.md) —
   `POST /task_by_id` and other mutations honor `read_write` shares.
6. [006-docs-and-manual-test-examples.md](006-docs-and-manual-test-examples.md) —
   README endpoint documentation and `.http` example files.
7. [007-username-lookup-by-id.md](007-username-lookup-by-id.md) — `GET
   /user_by_id` so clients can resolve the ids in task/share payloads to
   usernames.

Dependencies: 001 → 002 → {003, 004, 005} → 006. Ticket 007 is independent
and can be done any time after 001.

## Design decisions already made

- A new `task_shares` table records (task_id, user_id, permission) where
  permission is `read` or `read_write`.
- Existing listing endpoints (`/all_tasks`, `/incomplete_tasks`) include tasks
  shared with the requesting user, not just tasks they own.
- Only the task's owner may add/remove shares. Read-write grantees may view and
  update task fields but may not manage shares or (currently) delete tasks.
- Permission hierarchy: `owner` > `read_write` > `read` > none.
- Error responses must not distinguish "task does not exist" from "task exists
  but is not shared with you" (both 401), to avoid leaking task existence.

## Conventions for working on these tickets

- Core database logic lives in `src/core.rs`; HTTP handlers and request/response
  types live in `src/web_api.rs`; domain types with validation live in
  `src/domain_types.rs`.
- Schema changes are done with a new Diesel migration under `migrations/` and
  by regenerating `src/schema.rs` (`diesel migration run` / `diesel print-schema`).
- New endpoints must be registered in the `build_app!` macro in `src/lib.rs`.
- Every endpoint should have a tracing span and `event!` logging following the
  existing patterns, and return `{"request_id": ...}` in responses.
- Tests live in the `mod tests` block in `src/web_api.rs`; the database is
  wiped by the `#[ctor] global_init` before tests run.
- CI runs `cargo test` and `cargo clippy` — keep both clean.
