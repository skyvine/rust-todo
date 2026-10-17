# 009: An auth-key resource instead of `/login` and `/logout`

**Phase:** the redesign
**Depends on:** 005
**Files:** `src/web_api.rs`, `src/core.rs`, `README.md`, `http_requests/`

## Goal

`/login` and `/logout` name actions. The resource is an auth key, so it gets a
URI, and a created auth key gets a `Location`.

## Background

- `POST /login` (`web_api.rs:584-633`) takes HTTP Basic credentials in the
  `Authorization` header and inserts a row into `auth_keys`
  (`get_new_auth_key`, `core.rs:507-536`). It answers `200 OK` with the new key
  in the body and no `Location`, so the created resource is not addressable.
- `DELETE /logout` (`web_api.rs:253-271`) deletes the row for the key presented
  in the header (`logout`, `core.rs:631-646`) and answers `200` with an empty
  body.

Both names describe what the client wants rather than what exists. The
migration commits (`5cfe99c`, `721dd23`) already moved credentials and keys off
request bodies and into headers, which is the substance of this ticket; only
the URIs are left.

Note that the API stays stateless after this change: the auth key is still
presented in `Authorization: Bearer`, and `DELETE /auth_keys` just removes the
one the caller presented. Naming the resource does not introduce sessions.

## In scope

- `POST /auth_keys` — HTTP Basic credentials in, auth key out.
  - `201 Created` with `Location: /auth_keys/{key}` and the key as its
    representation, instead of `200` with the key buried in the body.
  - Ticket 004's uniform `401` applies here: unknown user and wrong password
    both answer `401` with the Basic challenge.
  - Ticket 002's challenge handling must keep the Basic challenge on this
    endpoint only.
- `DELETE /auth_keys` — revoke the key presented in the `Authorization` header.
  Replaces `/logout`. Answers `204 No Content` (ticket 014).
- `DELETE /auth_keys/{key}` should NOT be created.
- Keep the 401 behavior for a key that has already been revoked, and the
  expiration check in `auth_key_to_user` (`core.rs:394-423`) unchanged.

## Out of scope

- Key lifetime, expiry policy, refresh, or rotation.
- Returning a key that is not the presented one, or listing a user's keys.
- Changing the credential format. Basic stays; the server does not switch to
  cookies or form-encoded login, which would break the stateless design.
- Migrating `auth_keys` storage. Reusing the key value as the URI segment is
  acceptable for a secret that is already a random UUID, but see the note
  below.

## Acceptance criteria

- [ ] `POST /auth_keys` returns `201` with `Location` and the new key.
- [ ] `DELETE /auth_keys` revokes the presented key and returns `204`.
- [ ] After revocation, the key answers `401` on a Bearer-protected endpoint,
      with the bearer challenge.
- [ ] `/login` and `/logout` are gone.
- [ ] `/login`'s Basic challenge behavior is preserved at the new URI: missing,
      malformed, wrong-password, and unknown-user all answer `401` with
      `WWW-Authenticate: Basic realm="todo", charset="UTF-8"`.
- [ ] `login_password_may_contain_a_colon` still passes at the new URI.
- [ ] Tests updated: the `login` and `register_and_login` helpers,
      `login_requires_basic_authorization_header`,
      `refused_credentials_are_challenged_too`, `login_password_may_contain_a_colon`,
      `can_logout`, `invalid_password_fails`, `auth_key_is_recognized`.
- [ ] `http_requests/post_login.http` updated.
- [ ] `README.md` documents `/auth_keys`, including that Basic is still the
      credential format and that the key goes in `Authorization: Bearer`.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

`auth_keys` has no unique index on `key`
(`migrations/2025-07-06-170925_create-auth-keys/up.sql` declares `key text NOT
NULL` and nothing else); it works because keys are UUIDs. Worth confirming that
stays true if the key ever becomes addressable by a user-supplied value.
