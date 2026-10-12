# Ticket 005: Migrate `GET /task_by_id`

**Depends on:** ticket 001
**Files:** `src/web_api.rs`, `README.md`

## Goal

Change `GET /task_by_id` from a JSON body (`GetTaskByIdPayload`) to the
`Authorization: Bearer` header plus an `?id=<i32>` query parameter.

## Changes

- Add a query struct, e.g. `TaskByIdQuery { id: i32 }` (serde-deserializable by
  actix's `web::Query`).
- Replace `payload: actix_web::web::Json<GetTaskByIdPayload>` with the
  `AuthKey` extractor plus `web::Query<TaskByIdQuery>`; use the extracted key
  for `auth_key_to_user` and `query.id` for the task id.
- Delete the `GetTaskByIdPayload` struct.
- Update tests that call `/task_by_id`:
  - `can_get_task`
  - `cannot_get_different_users_task`
  - `cannot_get_nonexistent_task`
  - the `update_task` helper
  - `update_modifies_only_the_target_task`
  - `read_write_sharee_can_update_task_fields`
  - `read_only_sharee_cannot_update_task`
  - `unrelated_user_cannot_update_task`
  - `read_share_grants_get_and_listing_access`
  - `read_write_share_grants_get_and_listing_access`
  - `no_share_means_401_and_no_listing_entry`
  - Build requests with the `Authorization` header and `?id=<n>` in the URI
    instead of `.set_json(...)`.

## Documentation updates

- In `README.md`, update the `/task_by_id` entry: remove the `auth_key` input
  key, note the `Authorization: Bearer` header, and document the `id` query
  parameter (e.g. `GET /task_by_id?id=1`).

## Acceptance criteria

- [ ] `GetTaskByIdPayload` no longer exists.
- [ ] `/task_by_id` authenticates via the header and reads the id from the
      query string; response shape and 401-for-missing-task semantics are
      unchanged.
- [ ] A missing/non-numeric `id` returns `400` with the standard JSON error
      body.
- [ ] All tests touching `/task_by_id` pass.
- [ ] `README.md` `/task_by_id` section reflects the new header + query
      parameter input.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
