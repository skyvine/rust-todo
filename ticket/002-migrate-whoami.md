# Ticket 002: Migrate `GET /whoami`

**Depends on:** ticket 001
**Files:** `src/web_api.rs`, `README.md`

## Goal

Change `GET /whoami` from a JSON body (`WhoAmIPayload`) to the
`Authorization: Bearer` header.

## Changes

- Replace `payload: actix_web::web::Json<WhoAmIPayload>` with the `AuthKey`
  extractor from ticket 001; use the extracted key for `auth_key_to_user`.
- Delete the `WhoAmIPayload` struct.
- Update tests that call `/whoami`:
  - `auth_key_is_recognized`
  - `can_logout` (the post-logout whoami check)
  - Build requests with `.header(Authorization, format!("Bearer {auth_key}"))`
    instead of `.set_json(...)`.

## Documentation updates

- In `README.md`, update the `/whoami` entry: remove the `auth_key` input key
  and note that the auth key is sent via the `Authorization: Bearer` header.

## Acceptance criteria

- [ ] `WhoAmIPayload` no longer exists.
- [ ] `/whoami` authenticates via the header; response shape is unchanged.
- [ ] All tests touching `/whoami` pass.
- [ ] `README.md` `/whoami` section reflects the new header-based input.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
