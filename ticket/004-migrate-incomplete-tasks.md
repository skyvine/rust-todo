# Ticket 004: Migrate `GET /incomplete_tasks`

**Depends on:** ticket 001
**Files:** `src/web_api.rs`, `README.md`

## Goal

Change `GET /incomplete_tasks` from a JSON body (`GetIncompleteTasksPayload`)
to the `Authorization: Bearer` header.

## Changes

- Replace `payload: actix_web::web::Json<GetIncompleteTasksPayload>` with the
  `AuthKey` extractor; use the extracted key for `auth_key_to_user`.
- Delete the `GetIncompleteTasksPayload` struct.
- Update tests that call `/incomplete_tasks`:
  - `can_get_incomplete_tasks`
  - `read_share_grants_get_and_listing_access`
  - Build requests with the `Authorization` header instead of `.set_json(...)`.

## Documentation updates

- In `README.md`, update the `/incomplete_tasks` entry: remove the `auth_key`
  input key and note that the auth key is sent via the `Authorization: Bearer`
  header.

## Acceptance criteria

- [ ] `GetIncompleteTasksPayload` no longer exists.
- [ ] `/incomplete_tasks` authenticates via the header; response shape is
      unchanged.
- [ ] All tests touching `/incomplete_tasks` pass.
- [ ] `README.md` `/incomplete_tasks` section reflects the new header-based
      input.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
