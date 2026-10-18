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
  - `201 Created` with `Location: /auth_keys/{id}` and the key as its
    representation, instead of `200` with the key buried in the body.
  - `{id}` is the `auth_keys` primary key, **not** the key value. See "Why the
    URI carries the id" below.
  - Ticket 004's uniform `401` applies here: unknown user and wrong password
    both answer `401` with the Basic challenge.
  - Ticket 002's challenge handling must keep the Basic challenge on this
    endpoint only.
- `DELETE /auth_keys` — revoke the key presented in the `Authorization` header.
  Replaces `/logout`. Answers `204 No Content` (ticket 014).
- `DELETE /auth_keys/{id}` should NOT be created by this ticket.
- Keep the 401 behavior for a key that has already been revoked, and the
  expiration check in `auth_key_to_user` (`core.rs:394-423`) unchanged.

## Why the URI carries the id

The `Location` names the resource without being a second copy of the secret.
An auth key is a bearer token, and RFC 6750 §5.3 is explicit about where those
do not belong:

> Bearer tokens SHOULD NOT be passed in page URLs […] Browsers, web servers,
> and other software may not adequately secure URLs in the browser history, web
> server logs, and other data structures.

and §2.3 gives the reason — "including the high likelihood that the URL
containing the access token will be logged".

That advice is written about passing a token *as* the credential, which this
API does not do: the key travels in `Authorization: Bearer` throughout. But
`Location: /auth_keys/{key}` would put the same token in the request line of
every request to that URI, which is the exposure the advice is about, and it
would not be mitigable by any header — response headers govern whether a
*response body* is stored, and nothing governs whether an intermediary writes
the *request line* to its log.

`auth_keys` already has `id SERIAL PRIMARY KEY`, so the resource can be
identified the way every other resource in this API is: a task by its id, a
share by its username, a user by its id. Addressing the auth key by its own
secret would make it the only resource whose URI is a credential.

This also makes ticket 016's endpoint possible at all, and unblocks
`DELETE /auth_keys/{id}` — revoking a key you are not currently presenting,
which a URI naming the key value could never express.

## Out of scope

- Key lifetime, expiry policy, refresh, or rotation.
- Returning a key that is not the presented one, or listing a user's keys.
- Changing the credential format. Basic stays; the server does not switch to
  cookies or form-encoded login, which would break the stateless design.
- Migrating `auth_keys` storage. The key value stays the credential in the
  `Authorization` header and in the `201` body; only the URI changes. The
  missing unique index on `key` is ticket 016's to fix.

## Acceptance criteria

- [X] `POST /auth_keys` returns `201` with `Location` naming the new key's URI
      by its id, and the new key as the representation.
- [X] `DELETE /auth_keys` revokes the presented key and returns `204`.
- [X] After revocation, the key answers `401` on a Bearer-protected endpoint,
      with the bearer challenge.
- [X] `/login` and `/logout` are gone. Verified against a running server, which
      answers 404 for both; no test pins it, so nothing guards it against a
      route reappearing.
- [X] `/login`'s Basic challenge behavior is preserved at the new URI: missing,
      malformed, wrong-password, and unknown-user all answer `401` with
      `WWW-Authenticate: Basic realm="todo", charset="UTF-8"`.
- [X] `login_password_may_contain_a_colon` still passes at the new URI.
- [X] Tests updated: the `login` and `register_and_login` helpers,
      `login_requires_basic_authorization_header`,
      `refused_credentials_are_challenged_too`, `login_password_may_contain_a_colon`,
      `can_logout`, `invalid_password_fails`, `auth_key_is_recognized`.
- [X] `http_requests/post_login.http` updated.
- [X] `README.md` documents `/auth_keys`, including that Basic is still the
      credential format and that the key goes in `Authorization: Bearer`.
- [X] `cargo clippy` passes with no new warnings.
- [X] `cargo test` passes.
- [X] `cargo fmt --check` passes.

## Notes

`auth_keys` has no unique index on `key`
(`migrations/2025-07-06-170925_create-auth-keys/up.sql` declares `key text NOT
NULL` and nothing else); it works because keys are UUIDs. This stops being a
footnote when the key becomes an identifier in its own right: `auth_key_to_user`
filters by `key` and takes `results[0]`, so two rows sharing one key would
silently authenticate as whichever row loaded first. Ticket 016 adds the unique
index.

The id in the `Location` is the `auth_keys` primary key. It is enumerable, and
that is fine for the same reason task ids are: reading anything behind one
requires a valid auth key, and an id belonging to someone else is answered
exactly as an id that does not exist.
