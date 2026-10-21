# REST Constraint Violations — Ticket Breakdown

An audit of the API in `src/web_api.rs` against the REST architectural style
and the HTTP semantics that REST depends on. Findings map to tickets in the
[Findings map](#findings-map) below — nothing from the audit is left untracked.

Two facts about this API shape the whole plan:

- **There are no clients.** Every URI, request body, and response body can
  change in one release. There is no deprecation window to honor, no versioning
  prefix to add, and no alias to keep. That removes the usual reason to stage
  this work — and it shapes the *order*: several conformance tickets would
  otherwise be written against the old URIs and then thrown away by the
  redesign, so those are numbered after it.
- **Account enumeration is not a concern.** Ticket 004 is therefore a
  status-code correctness fix, not a hardening fix. Note that the
  existence-oracle rule for *tasks* is a different question and is preserved
  throughout.

## Tickets

Numbers are completion order: work them lowest to highest. Every `Depends on`
line points at a lower number, so no ticket is ever blocked by a later one.

| # | Ticket | Summary |
|---|--------|---------|
| 001 | [json-content-type-on-every-response.md](001-json-content-type-on-every-response.md) | Every response body is JSON but no response carries a `Content-Type`. |
| 002 | [www-authenticate-challenge-on-401.md](002-www-authenticate-challenge-on-401.md) | 401 responses on Bearer-protected endpoints omit the mandatory challenge. |
| 003 | [403-for-authorization-failures.md](003-403-for-authorization-failures.md) | Authenticated-but-not-permitted is reported as 401, which claims the credentials are invalid. |
| 004 | [401-for-unknown-username-on-login.md](004-401-for-unknown-username-on-login.md) | An unknown username answers 404, which is the wrong status for a refused authentication attempt. |
| 005 | [plan-resource-oriented-uris.md](005-plan-resource-oriented-uris.md) | Plan and decisions; pins the URI scheme that 006–010 implement. |
| 006 | [tasks-as-a-resource.md](006-tasks-as-a-resource.md) | One task resource, one URI, id in the path. |
| 007 | [single-task-collection-with-filter.md](007-single-task-collection-with-filter.md) | `/all_tasks` and `/incomplete_tasks` are one collection, not two. |
| 008 | [shares-as-a-sub-resource.md](008-shares-as-a-sub-resource.md) | Shares as a sub-resource of a task; removes the `DELETE`-with-body violation and gives share upsert `PUT` semantics. |
| 009 | [auth-key-resource-replacing-login-and-logout.md](009-auth-key-resource-replacing-login-and-logout.md) | `/login` and `/logout` name actions; the resource is an auth key. |
| 010 | [users-as-a-resource.md](010-users-as-a-resource.md) | `/whoami`, `/user_by_id`, `/register_account` name RPCs; the resource is a user. |
| 011 | [location-header-on-201-created.md](011-location-header-on-201-created.md) | Every `201 Created` must identify the new resource, and the core functions must be able to report what they created. |
| 012 | [405-and-allow-for-unsupported-methods.md](012-405-and-allow-for-unsupported-methods.md) | A known path with an unsupported method answers 404 instead of 405. |
| 013 | [etag-and-conditional-requests.md](013-etag-and-conditional-requests.md) | No cache validators and no conditional requests, so concurrent edits clobber. |
| 014 | [request-id-out-of-representations.md](014-request-id-out-of-representations.md) | `request_id` is embedded in every representation, no URI returns its own resource, and an error helper builds success bodies. |
| 015 | [docs-and-http-examples-for-the-new-uris.md](015-docs-and-http-examples-for-the-new-uris.md) | Docs and `http_requests/` sweep. |
| 016 | [addressable-auth-key-with-expiry.md](016-addressable-auth-key-with-expiry.md) | The `Location` of a created auth key names a URI nothing is served at, and expiry is invisible. |
| 017 | [get-a-single-share.md](017-get-a-single-share.md) | The `Location` of a created share names a URI nothing is served at. |
| 018 | [head-is-answered-where-get-is.md](018-head-is-answered-where-get-is.md) | A path which serves `GET` does not serve `HEAD`, so a `HEAD` request is refused. |
| 019 | [options-answers-with-the-methods-a-path-supports.md](019-options-answers-with-the-methods-a-path-supports.md) | `OPTIONS` is refused rather than answered with the methods a path supports. |

The numbering has five phases, which is why 011–014 come after the redesign
even though they are smaller than the redesign tickets:

- **001–004** change media types, headers, and status codes but no URIs, so
  they carry over unchanged. Doing them first means the redesign lands on a
  conformant API instead of adding fresh nonconformity.
- **005–010** are the redesign itself.
- **011–014** are the conformance fixes that *have* to be written against the
  final URIs — a `Location` header, the path/method table behind 405, and the
  response-shape change each apply to every endpoint, so writing them against
  the old URIs would mean writing them twice.
- **016** is a loose end 009 left open rather than a new finding: 009 minted a
  `Location` for the auth key and scoped out every method on it, so the URI is
  real but unserved. It lands late because the `405` it inherits for every other
  method should come from 012's mechanism. It is also where 009's "the key value
  may be the URI segment" decision gets reversed — see
  [009's "Why the URI carries the id"](009-auth-key-resource-replacing-login-and-logout.md).
- **017** is the same kind of loose end, found by 011 rather than left by it:
  008 minted a `Location` for a created share and scoped out a `GET` on it, so
  the URI is real but unserved. It is a smaller sibling of 016, and it depends
  on the same 012 mechanism for the path's `Allow`.
- **018 and 019** are gaps 012 found rather than left by it: 012 makes a path
  answer `405` for a method it does not serve, and in doing so it made visible
  that the API serves no `HEAD` and no `OPTIONS` at all. Both land after 012,
  because both are answers the path's `Allow` header has to keep true.

Dependencies:

- 001, 002, 003, 004 are independent of each other; 002 before 003, since both
  rewrite the 401 paths.
- 005 must land before 006–010.
- 006 → 007 → 008. 009 and 010 are independent of each other and of 006–008.
- 011, 012, 013, 014 apply to the whole API after 006–010 and are independent
  of each other; 015 depends on all of them.
- 016 depends on 009 and 012. 015 does not depend on 016: the README it writes
  describes the API as it stands, and 016 changes one endpoint of it, so 015 can
  land first and 016 updates that section the way every other ticket does.
- 017 depends on 008, 011 and 012, for the same reasons 016 does.
- 018 and 019 depend on 012, and are independent of each other.

## Findings map

| # | Finding | Ticket |
|---|---------|--------|
| 1 | No `Content-Type` on JSON responses | 001 |
| 2 | 401 without `WWW-Authenticate` on Bearer endpoints | 002 |
| 3 | 201 without `Location`; no representation of what was created | 011 |
| 4 | 401 used for authorization failures (403 is correct) | 003 |
| 5 | 404 for unknown username at `/login` | 004 |
| 6 | 404 instead of 405 + `Allow` for unsupported methods | 012 |
| 7 | No `ETag`, `Cache-Control`, `Vary`, or `If-Match` | 013 |
| 8 | `request_id` in every representation; no URI returns its resource; error body reused for success | 014 |
| 9 | RPC-style verb URIs (`/add_task`, `/logout`, …) | 006, 008, 009, 010 |
| 10 | Resource ids in query string / body instead of the URI | 006, 007, 008, 010 |
| 11 | One resource (`tasks`) under two URIs | 007 |
| 12 | `POST /share_task` is an idempotent upsert, so it is `PUT` | 008 |
| 13 | `DELETE /unshare_task` carries a request body | 008 |
| 14 | Stale `README.md` / `http_requests/` (send `auth_key` in bodies) | 015 |
| 15 | A `Location` naming a URI nothing is served at (a created auth key) | 016 |
| 16 | A `Location` naming a URI nothing is served at (a created share) | 017 |
| 17 | No `HEAD` is served where `GET` is | 018 |
| 18 | No `OPTIONS` is served anywhere | 019 |

## Conventions for working on these tickets

- HTTP handlers, extractors, and request/response types live in
  `src/web_api.rs`; core application and database logic in `src/core.rs`;
  domain types with validation in `src/domain_types.rs`.
- Every new or changed endpoint must be registered in the `build_app!` macro
  in `src/lib.rs`.
- Follow the existing response conventions: a tracing span carrying a
  `request_id`, `event!` logging at the existing levels, and the
  `error_body` / `error_body_with_message` helpers for error bodies (until
  ticket 014 replaces part of that).
- Update `README.md` and the matching file in `http_requests/` **in the same
  ticket** as the code change, not in a follow-up — except for ticket 015,
  which is the final sweep.
- Tests live in the `mod tests` block in `src/web_api.rs`; the database is
  wiped by `#[ctor] global_init` before the suite runs, so tests share one
  database and cannot run in isolation reliably.

## Verification discipline

Every ticket must leave the repository green. Before a ticket is considered
complete, and before any commit is made:

- `cargo clippy` — no new warnings.
- `cargo test` — passes. Requires a running PostgreSQL: start one with
  `scripts/run_test_database.sh`, then run with
  `DATABASE_URL=postgres://postgres:test-password@localhost/todo`.

Some tickets change status codes that existing tests assert on. Those
assertions are part of the ticket's work — do not relax the test to make it
pass, update the test to the corrected expectation and say so in the
acceptance criteria.

## Design decisions already made

- **Authentication stays stateless.** Auth keys continue to travel in the
  `Authorization: Bearer` header (the work in commits `71dcf0c`…`431acd1`).
  The redesign only renames the URIs; it does not reintroduce cookies or
  sessions.
- **Credentials keep using HTTP Basic**, and only move from `/login` to
  `POST /auth_keys`; they never go in a body.
- **Hard cut, no versioning.** Since there are no clients, the old URIs are
  deleted rather than aliased. Permanent aliases are doubly wrong here: they
  would keep two URIs per resource, which is the problem being fixed.
- **No deprecation windows.** Nothing is kept "for one release" — a response
  field that is merely deprecated is still a field to maintain, and there is
  nobody to transition.
- **Task existence stays hidden.** "Task does not exist" and "task exists but
  you may not see it" must return the same status and body shape. This is
  about task ids, not account names, and it is unaffected by the decision that
  enumeration of *accounts* is not a concern.
- **Key existence stays hidden too, and this one is not a style choice.** Unlike
  an account name, a leaked key is a live credential: "this key exists" and "this
  key does not exist" must be one answer, so `GET /auth_keys/{id}` describes a
  key only to the user who owns it and says nothing at all about anyone else's
  (ticket 016). No expiry or ownership information leaves the owner's own view.
- **An auth key is addressed by its row id, never by its value.** The key is a
  bearer token, and RFC 6750 §5.3 says those do not belong in URLs — every proxy
  and log that records a request line records the credential. So
  `Location: /auth_keys/{id}`, the same way a task is addressed by its id and a
  share by its username (tickets 009, 016). The key value stays in the
  `Authorization` header and in the `201` body, and gets a unique index so a
  duplicate cannot make one credential resolve to two users.
- **User ids are public** (they appear in task payloads), so
  `GET /users/{id}` keeps returning 404 for an unknown id.
- **Account enumeration is accepted.** `/login` answers 401 for an unknown
  username (ticket 004) because 404 is the wrong status, not to hide the
  account. `POST /users` keeps its 409 on a duplicate name for the same
  reason. No work is spent making either path constant-time.
- **`PATCH` is already correct in spirit** — partial updates of a task
  resource — and keeps its method through the redesign; only its URI changes.