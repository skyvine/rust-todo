# 007: Look up a username by user id

## Goal

Let a user resolve a user id (as exposed in `Task`'s `owner` field, and in
share responses) to a username, without changing the serialized shape of
existing responses.

## Background

After tickets 001–006, tasks shared with me appear in my task listings, and
each entry's `owner` field names the creating user's **id**, not their
username. Ticket 004's out-of-scope note deferred adding a `shared_by` field
to task responses. This ticket provides the same discoverability via a lookup
endpoint instead: a client can take `task.owner` from a listing and resolve it
to a username on demand, instead of every task payload carrying the extra
field.

Existing building blocks:

- `users` table: `id`, `username`, `password` (`src/schema.rs`).
- `get_user_by_name` in `src/core.rs` is the model for a by-field user
  lookup; it maps a miss to `ApplicationError::DieselError(Error::NotFound)`.
- `auth_key_to_user` pattern in handlers for authentication with a request-id
  span (`src/web_api.rs`).

## In scope

- Add core function `get_user_by_id(user_id: &i32, connection: &mut
  PgConnection) -> Result<User, ApplicationError>` in `src/core.rs`, mirroring
  `get_user_by_name` (returns `DieselError(Error::NotFound)` when absent).
- Add `GET /user_by_id` endpoint in `src/web_api.rs`:
  - Input keys: `auth_key` [string], `id` [number].
  - Output keys on success: `username` [string], `request_id`.
  - Unknown id returns 401, same body shape as an invalid auth key
    (existence-oracle rule in `tickets/README.md`) — not 404.
- Register the endpoint in the `build_app!` macro in `src/lib.rs`.
- Integration tests in `mod tests` in `src/web_api.rs`: lookup of an existing
  user returns their username; lookup of an unknown id returns 401.

## Out of scope

- Changing `Task`'s serialized fields to embed usernames.
- Batch resolution of many ids in one request (can be a follow-up if
  clients need it).

## Note: enumeration tradeoff

A strict version would only resolve ids the caller is connected to through a
share (e.g. the caller owns a task shared with that user, or vice versa). This
ticket's straightforward version returns the username for any existing id.
Discuss/decide before implementing; if restricted, update the acceptance
criteria accordingly.

## Acceptance criteria

- `GET /user_by_id` returns the correct username for an existing user id and a
  401 for an unknown id.
- `cargo test` and `cargo clippy` pass.
- README and `http_requests/` are updated (ticket 006's checklist style —
  add the docs in the same PR if 006 has already landed).
