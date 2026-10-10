# Plan: Migrate GET endpoints from JSON bodies to query parameters + headers

## Summary

Move the 6 authenticated GET endpoints off JSON bodies:

- The secret (`auth_key`) moves to an `Authorization: Bearer <auth_key>` header.
- The non-secret `id` moves to an `?id=<i32>` query parameter.
- `GET /is_alive` takes no input and is unchanged.
- Scope is GET endpoints only; POST/DELETE endpoints keep their JSON bodies.

## Design decisions

1. **Auth header**: `Authorization: Bearer <auth_key>` (standard HTTP convention).
2. **Query parameter**: named `id`, matching the current JSON field name.
3. **Missing/malformed `Authorization` header** → `401` with the standard JSON error body (`{"request_id": ...}`), identical to today's invalid-key response.
4. **Missing/malformed `id` query parameter** → `400` with the standard JSON error body. Actix's `Query` extractor rejects the request; a custom `QueryConfig` error handler keeps the body consistent with the rest of the API instead of actix's plain-text default.
5. **Zeroization preserved**: the extracted auth key is held in a `ZeroizeOnDrop` newtype, matching the existing `ZeroizeOnDrop` payload structs.
6. **Responses are unchanged**: still JSON, still carrying `request_id`.

## Endpoint mapping

| Endpoint          | Current input          | New input            |
|-------------------|------------------------|----------------------|
| `GET /whoami`     | JSON `{auth_key}`      | header only          |
| `GET /all_tasks`  | JSON `{auth_key}`      | header only          |
| `GET /incomplete_tasks` | JSON `{auth_key}` | header only          |
| `GET /task_by_id` | JSON `{auth_key, id}`  | header + `?id=`      |
| `GET /user_by_id` | JSON `{auth_key, id}`  | header + `?id=`      |
| `GET /task_shares`| JSON `{auth_key, id}`  | header + `?id=`      |
| `GET /is_alive`   | —                      | unchanged            |

## Implementation steps

1. **Shared extractor** (ticket 001): add a `BearerAuth` extractor and `AuthKey` newtype in `src/web_api.rs`, plus a `QueryConfig` error handler registered in `build_app!`.
2. **Per-endpoint migration** (tickets 002–007): swap `web::Json<XPayload>` for `BearerAuth` (+ `web::Query<...>` where an `id` is needed), delete the old payload structs, update every test that calls the endpoint, and update the corresponding README section and `.http` request file in the same ticket.
3. **Verify** (ticket 008): `cargo fmt`, `cargo clippy`, `cargo test`.

## Verification discipline

- Every ticket must leave the repository in a fully green state. Before a
  ticket is considered complete (and before any commit is made), all of the
  following must pass:
  - `cargo fmt --check`
  - `cargo clippy` (no new warnings)
  - `cargo test`
- No commit may introduce a failing state.

## Risks / notes

- This is a breaking change for existing clients; coordinate the rollout.
- Actix's default extractor errors are plain text; the custom error handlers keep error bodies consistent JSON.
- The test suite shares one database and wipes it in a `#[ctor]`; keep that pattern untouched.
