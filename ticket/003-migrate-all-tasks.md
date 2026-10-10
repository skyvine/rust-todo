# Ticket 003: Migrate `GET /all_tasks`

**Depends on:** ticket 001
**Files:** `src/web_api.rs`, `README.md`

## Goal

Change `GET /all_tasks` from a JSON body (`GetAllTasksPayload`) to the
`Authorization: Bearer` header.

## Changes

- Replace `payload: actix_web::web::Json<GetAllTasksPayload>` with the
  `BearerAuth` extractor; use the extracted key for `auth_key_to_user`.
- Delete the `GetAllTasksPayload` struct.
- Update tests that call `/all_tasks`:
  - `can_get_all_tasks`
  - `read_share_grants_get_and_listing_access`
  - `read_write_share_grants_get_and_listing_access`
  - `no_share_means_401_and_no_listing_entry`
  - `owner_listings_never_duplicate_with_stale_share_row`
  - Build requests with the `Authorization` header instead of `.set_json(...)`.

## Documentation updates

- In `README.md`, update the `/all_tasks` entry: remove the `auth_key` input
  key and note that the auth key is sent via the `Authorization: Bearer`
  header.

## Acceptance criteria

- [ ] `GetAllTasksPayload` no longer exists.
- [ ] `/all_tasks` authenticates via the header; response shape is unchanged.
- [ ] All tests touching `/all_tasks` pass.
- [ ] `README.md` `/all_tasks` section reflects the new header-based input.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
