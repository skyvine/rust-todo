# Ticket 006: Migrate `GET /user_by_id`

**Depends on:** ticket 001
**Files:** `src/web_api.rs`, `README.md`, `http_requests/get_user_by_id.http`

## Goal

Change `GET /user_by_id` from a JSON body (`GetUserByIdPayload`) to the
`Authorization: Bearer` header plus an `?id=<i32>` query parameter.

## Changes

- Add a query struct, e.g. `UserByIdQuery { id: i32 }`.
- Replace `payload: actix_web::web::Json<GetUserByIdPayload>` with the
  `AuthKey` extractor plus `web::Query<UserByIdQuery>`; use the extracted key
  for `auth_key_to_user` and `query.id` for the user id.
- Delete the `GetUserByIdPayload` struct.
- Update tests that call `/user_by_id`:
  - `user_by_id_returns_username_for_existing_user`
  - `user_by_id_returns_404_for_unknown_id`
  - Build requests with the `Authorization` header and `?id=<n>` in the URI
    instead of `.set_json(...)`.

## Documentation updates

- In `README.md`, update the `/user_by_id` entry: remove the `auth_key` input
  key, note the `Authorization: Bearer` header, and document the `id` query
  parameter (e.g. `GET /user_by_id?id=1`).
- Update `http_requests/get_user_by_id.http`: replace the JSON body with an
  `Authorization: Bearer <key>` header and `?id=1` in the request line; remove
  the now-meaningless `Content-Type`/`Content-Length` headers.

## Acceptance criteria

- [ ] `GetUserByIdPayload` no longer exists.
- [ ] `/user_by_id` authenticates via the header and reads the id from the
      query string; response shape and 404-for-unknown-id semantics are
      unchanged.
- [ ] A missing/non-numeric `id` returns `400` with the standard JSON error
      body.
- [ ] All tests touching `/user_by_id` pass.
- [ ] `README.md` `/user_by_id` section reflects the new header + query
      parameter input.
- [ ] `http_requests/get_user_by_id.http` uses the header + query string.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
