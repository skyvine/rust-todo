# 015: Docs and manual-request sweep for the new URIs

**Phase:** docs sweep
**Depends on:** 011, 012, 013, 014
**Files:** `README.md`, `http_requests/`

## Goal

The README describes the API that actually exists, and the manual request
files request real endpoints.

## Background

The README and `http_requests/` are updated per-ticket as code changes, which
has left them describing an API that no longer exists:

- `README.md` documents `auth_key` as a body input key for `/add_task`
  (lines 91-97), `/share_task` (98-103), and `/unshare_task` (32-36). The code
  moved that to the `Authorization` header in commits `71dcf0c` and
  `431acd1`, so those keys are now silently ignored.
- `http_requests/delete_unshare_task.http` and
  `http_requests/post_share_task.http` still send `"auth_key": "manual-auth-key"`
  in the body — the same dead key. Their `Authorization` headers are missing
  entirely, so both requests would now be rejected at authentication.
- `http_requests/get_task_shares.http` and `get_user_by_id.http` are missing
  the `Authorization` header that `get_is_alive.http` does not need.
- The `Login` entry documents the challenge behavior added in `cde4081`, which
  is good; the behavior changed by ticket 004 (unknown user is a 401, not a
  404) is documented nowhere.
- The README groups endpoints by HTTP method. Once the URIs are
  resource-oriented, grouping by resource reads better and shows the whole life
  cycle of each one.

This ticket is the sweep that makes the documentation trustworthy once the
redesign has landed. It is separate because each prior ticket touches only its
own section, and a final pass catches what per-ticket edits miss.

## In scope

- Restructure `README.md`'s "Endpoints" section by resource, so each resource
  shows its collection, item, and sub-resource routes together:
  - `/tasks` (`GET`, `POST`, and `GET`/`PATCH`/`DELETE` on `/tasks/{id}`)
  - `/tasks/{id}/shares` (`GET`, `PUT`, `DELETE`)
  - `/users` (`POST`, `GET /users/{id}`, `GET /users/me`)
  - `/auth_keys` (`POST`, `DELETE`)
  - `/health`
- For each endpoint, document: the URI, the method, the media type in both
  directions, every status it can return with what triggers it, and any
  conditional-request or header requirements from tickets 002, 011, 003, 013,
  and 014. The current README documents input keys and output keys but not
  status codes, which is why several of the conformance bugs went unnoticed.
- State the authentication model once, at the top: Basic for credential
  exchange, `Bearer` for everything else, `WWW-Authenticate` on 401, and
  `X-Request-Id` on every response. Do not repeat it per endpoint.
- Rewrite every file in `http_requests/` so each is a working request against
  the new URIs, with the correct `Authorization` header and no dead
  `auth_key` body keys. Confirm each one by running it.
- Since the README now describes the only version of the API that ever existed,
  it does not need a migration guide or a compatibility table. If the endpoint
  history is worth recording, put it in a commit message or an ADR rather than
  burdening the reference docs.

## Out of scope

- OpenAPI/Swagger generation, or publishing a machine-readable schema. Worth
  considering once the URIs stop changing; it would also have caught the
  missing `Content-Type` and `Location` headers.
- A hosted documentation site, changelog automation, or release tooling.
- Any code change. If the docs reveal a behavior that is wrong, file or fix it
  in its own ticket rather than patching it here.

## Acceptance criteria

- [ ] Every endpoint in `README.md` exists, and every endpoint in the code
      appears in `README.md`.
- [ ] No document mentions `auth_key` as a request-body key.
- [ ] Every endpoint's status codes are documented, including 401 vs 403 and
      the 201-vs-200 rule for share creation.
- [ ] Every file in `http_requests/` was executed against a running server and
      returned the documented status.
- [ ] The README contains no reference to endpoints that no longer exist.
- [ ] A reader who has never seen the API can register, log in, create a task,
      share it, and update it using only the README.

## Notes

Running every `.http` file against a live server is the step that keeps this
honest. Start the database with `scripts/run_test_database.sh`, run the server
with `DATABASE_URL=postgres://postgres:test-password@localhost/todo`, and
check each response.