# Ticket 007: Migrate `GET /task_shares`

**Depends on:** ticket 001
**Files:** `src/web_api.rs`, `README.md`, `http_requests/get_task_shares.http`

## Goal

Change `GET /task_shares` from a JSON body (`GetTaskSharesPayload`) to the
`Authorization: Bearer` header plus an `?id=<i32>` query parameter.

## Changes

- Add a query struct, e.g. `TaskSharesQuery { id: i32 }`.
- Replace `payload: actix_web::web::Json<GetTaskSharesPayload>` with the
  `BearerAuth` extractor plus `web::Query<TaskSharesQuery>`; use the extracted
  key for `auth_key_to_user` and `query.id` for the task id.
- Delete the `GetTaskSharesPayload` struct.
- Update the `get_task_shares` test helper and every test that calls
  `/task_shares`:
  - `owner_can_share_and_see_share_listed`
  - `resharing_changes_the_permission`
  - `non_owner_cannot_share_unshare_or_list`
  - `unsharing_removes_the_row`
  - `missing_task_returns_same_401_as_not_owned`
  - `read_write_sharee_cannot_manage_shares`
  - Build requests with the `Authorization` header and `?id=<n>` in the URI
    instead of `.set_json(...)`.

## Documentation updates

- In `README.md`, update the `/task_shares` entry: remove the `auth_key` input
  key, note the `Authorization: Bearer` header, and document the `id` query
  parameter (e.g. `GET /task_shares?id=1`).
- Update `http_requests/get_task_shares.http`: replace the JSON body with an
  `Authorization: Bearer <key>` header and `?id=1` in the request line; remove
  the now-meaningless `Content-Type`/`Content-Length` headers.

## Acceptance criteria

- [ ] `GetTaskSharesPayload` no longer exists.
- [ ] `/task_shares` authenticates via the header and reads the id from the
      query string; response shape and 401 semantics are unchanged.
- [ ] A missing/non-numeric `id` returns `400` with the standard JSON error
      body.
- [ ] All tests touching `/task_shares` pass.
- [ ] `README.md` `/task_shares` section reflects the new header + query
      parameter input.
- [ ] `http_requests/get_task_shares.http` uses the header + query string.
- [ ] `cargo fmt --check` passes.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
