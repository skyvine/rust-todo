# 010: A user resource instead of `/register_account`, `/whoami`, `/user_by_id`, and `/is_alive`

**Phase:** the redesign
**Depends on:** 005
**Files:** `src/web_api.rs`, `src/core.rs`, `README.md`, `http_requests/`

## Goal

Users are addressed as resources, and the two endpoints that are not about
users at all get names that do not impersonate RPCs.

## Background

- `POST /register_account` (`web_api.rs:647-694`) creates a user. The URI names
  an operation, not a resource. It returns `201` with an empty body, which
  ticket 011 fixes.
- `GET /whoami` (`web_api.rs:502-526`) returns the caller's username. This is
  `GET /users/me` — a URI naming a resource relative to the caller, which is
  exactly what `/users/me` means.
- `GET /user_by_id?id=N` (`web_api.rs:468-499`) resolves a user id that appears
  in task and share payloads. The id belongs in the path.
- `GET /is_alive` (`web_api.rs:408-412`) is a liveness probe. Not a resource,
  but `/is_alive` reads like a boolean field rather than an endpoint; `/health`
  is the convention every deployment tool already understands.

`/whoami` and `/user_by_id` overlap: the caller has an id, so `/users/me` and
`GET /users/{id}` are the same resource under two URIs unless `/users/me` is
special-cased. That special case is standard and worth keeping, since
resolving your own id through a numeric id would be silly.

## In scope

- `POST /users` — create an account. Body `{username, password}` as today.
  Returns `201` with `Location: /users/{id}` (per ticket 011's rule) and a
  representation containing the username, never the hash.
- `GET /users/me` — replaces `/whoami`. Returns the caller's username, and
  consider returning the id too, since it appears in every task payload and
  clients otherwise need `/users/{id}` to discover their own.
  - **Decision**: Return the id too.
- `GET /users/{id}` — replaces `/user_by_id?id=N`. Keeps returning `404` for an
  unknown id, per the decision recorded in `tickets/README.md` that user ids are
  public.
- `GET /health` — replaces `/is_alive`. Keep it unauthenticated and keep
  answering `200` with an empty body.
- Route ordering must ensure `/users/me` is matched before `/users/{id}`, or
  that `me` is rejected as a non-numeric id. Actix matches in registration
  order, so register `/users/me` first and cover the behavior with a test.

## Out of scope

- User deletion, profile editing, password change, or an email field. A
  `PATCH /users/{id}` for the username alone could come later; deciding the
  status for changing a username (which share rows reference) is a separate
  design question.
- Listing users. There is no such endpoint today and no need for one; adding
  `GET /users` would turn "look up a user whose id you have" into "browse every
  account", which is a new capability rather than a new spelling of an existing
  one.
- The username-existence signal from `409 Conflict` on duplicate registration,
  which is noted in ticket 004.

## Acceptance criteria

- [x] `POST /users`, `GET /users/me`, `GET /users/{id}`, and `GET /health`
      exist; `/register_account`, `/whoami`, `/user_by_id`, and `/is_alive` are
      gone.
- [x] `GET /users/me` returns the caller's username and is not shadowed by the
      `{id}` route.
- [x] `GET /users/{id}` returns `404` for an unknown id, as
      `GET /user_by_id?id=N` does today.
- [x] `GET /health` is unauthenticated and returns `200` with an empty body.
- [x] No response body ever contains a password hash.
- [x] Tests updated: the `register` helper,
      `regitering_account_is_successful`, `duplicate_username_fails`,
      `auth_key_is_recognized` (which uses `/whoami`),
      `user_by_id_returns_username_for_existing_user`,
      `user_by_id_returns_404_for_unknown_id`,
      `cannot_get_user_without_a_valid_id_query_parameter` (repurpose or
      delete), `is_alive` (which asserts the empty body).
- [x] `README.md` has a users section and a health section, documenting
      `/users/me` as the caller-relative resource.
- [x] `http_requests/post_registration.http`, `get_user_by_id.http`,
      `get_is_alive.http` updated.
- [x] `cargo clippy` passes with no new warnings.
- [x] `cargo test` passes.
- [x] `cargo fmt --check` passes.

## Notes

`User` (`core.rs:245-249`) does not derive `Serialize`, so a response struct is
needed. Serialize only `id` and `username` — never `password`.
