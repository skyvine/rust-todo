# 016: An addressable auth key, with its expiry, readable only by its owner

**Phase:** after the redesign — it closes a scope decision ticket 009 left open
**Depends on:** 009, 012
**Files:** `src/web_api.rs`, `src/core.rs`, `src/lib.rs`, `migrations/`, `README.md`, `http_requests/`

## Goal

`POST /auth_keys` answers `201 Created` with `Location: /auth_keys/{id}`, and
nothing is served at that URI. Serve the key's representation there — including
when it expires — and only to the user the key belongs to.

## Background

- Ticket 009 scoped out `DELETE /auth_keys/{id}` and created no `GET`, so no
  route matches `/auth_keys/{id}` at all and any request to it is answered `404`
  by actix's default service. The API therefore mints a URI in a `Location` and
  then declines to recognize it when addressed. Verified against a running
  server.
- 009's reasoning — the API never hands out a key other than the presented one —
  still holds for *acting* on a key: revoking is done by presenting it. It does
  not hold for *describing* one. Every `POST /auth_keys` mints a new key and
  nothing rotates them away, so a user accumulates keys and has no way to ask
  when any of them expires.
- Expiry is already the reason a key stops working: `auth_key_to_user` filters
  on `expiration > now()`, which makes an expired key indistinguishable from an
  unknown one on every Bearer-protected endpoint. A client whose key has expired
  gets `401` and is never told when it happened.
- **`expiration` cannot be reported as the column stands.**
  `migrations/2025-07-06-170925_create-auth-keys/up.sql` declares
  `expiration timestamp NOT NULL DEFAULT (now() + interval '30' day)`, i.e.
  `timestamp without time zone`, so the stored value is wall-clock in whatever
  zone the writing session used. In the test database:
  ```
  TimeZone    = America/Los_Angeles
  expiration  = 2026-11-17 15:09:59.545435
  ```
  Diesel reads that as a `NaiveDateTime` carrying no zone, so serializing it as
  RFC 3339 with a `Z` would be **eight hours wrong**. The same column is already
  compared against `now()` (a `timestamptz`) in `auth_key_to_user`, which
  Postgres coerces with the session zone — self-consistent only for as long as
  that zone stays put.

## In scope

- `GET /auth_keys/{id}`, answering `200` with the key's representation to its
  owner, whether or not the key has expired.
- A migration making the `expiration` value reportable, and the key unique.
- `401` behavior on Bearer-protected endpoints, and the expiration check in
  `auth_key_to_user`, unchanged.

## Decisions

1. **The URI carries the `auth_keys` primary key, not the key value.** The key
   is a bearer token, and RFC 6750 §5.3 is explicit about where those do not
   belong:
   > Bearer tokens SHOULD NOT be passed in page URLs […] Browsers, web servers,
   > and other software may not adequately secure URLs in the browser history,
   > web server logs, and other data structures.

   with §2.3 giving the reason: "including the high likelihood that the URL
   containing the access token will be logged". That advice is written about
   passing a token *as* the credential, which this API never does — the key
   travels in `Authorization: Bearer` — but a URI segment carrying the key puts
   it in the request line of every request, which is the exposure the advice is
   about, and no header mitigates it: response headers govern whether a
   *response body* is stored, and nothing governs whether an intermediary writes
   the *request line* to its log. The id is already there, and it makes the auth
   key the only resource in this API whose URI is not a secret: a task is
   addressed by id, a share by username, a user by id.
2. **Only the owning user may read a key.** The caller's own key establishes who
   they are: the `Authorization: Bearer` header goes through
   `auth_key_to_user` exactly as on every other Bearer-protected endpoint, so a
   missing, malformed, unknown or expired key is a `401` with the bearer
   challenge. The named key is then looked up **without** the expiration filter,
   and its `user_id` must equal the caller's. A caller who holds another valid
   key may therefore describe any of their own keys, and nobody else's.
3. **A key which is not the caller's and a key which does not exist are answered
   identically** — `404` with the standard `{"request_id": ...}` error body.
   `404` rather than the `403` the task endpoints use for the same class of
   hiding, because the client can never act on a key that is not theirs either
   way, and §15.5.5's "no current representation for the target resource **or is
   not willing to disclose that one**" describes what actually happened. The task
   endpoints chose `403` because their caller may legitimately hold the resource;
   that argument does not transfer. See Notes for flipping it.
4. **An owned key is described even once it has expired**, which is the whole
   point of exposing the expiry. A caller whose *only* key has expired gets `401`
   and cannot read anything: the endpoint needs a live credential to say whose
   credential is being asked about. That limitation is accepted, not worked
   around.
5. **The representation is `id`, `username`, `expiration`, and the key value.**
   The key belongs in it: it is the resource's actual content, the caller has
   just proved they own it with a bearer credential, and — unlike a URI — its
   disclosure is bounded by `Cache-Control: no-store`. It is the same string the
   client already holds, so this reveals nothing to its owner; it is what lets a
   client which stored the id fetch the credential back after a local loss.
6. **`expiration` is RFC 3339 with an explicit offset**, which is what forces the
   migration:
   ```sql
   ALTER TABLE auth_keys ALTER COLUMN expiration TYPE timestamptz;
   ```
   Postgres converts using the session zone, so existing rows are read correctly
   as long as the database has run in one zone; the ticket is done when that is
   no longer a thing worth writing down. The `now() + interval '30' day` default
   is unambiguous on a `timestamptz` column.
7. **`auth_keys.key` gets a unique index**, which is no longer only about URI
   uniqueness: `auth_key_to_user` filters by `key` and takes `results[0]`, so two
   rows sharing a key would silently authenticate as whichever row loaded first.
   `get_new_auth_key` mints a UUIDv4, so no existing row can collide; the index
   exists so a collision is refused rather than resolved at random.
   ```sql
   CREATE UNIQUE INDEX auth_keys_key_unique ON auth_keys (key);
   ```

## Leak vectors

The point of this ticket is as much about what must *not* escape as about the
endpoint.

| Vector | What it would leak | Closed by |
|---|---|---|
| Existence of someone else's key | confirms to an enumerator that an id is live, and by the shape of the answer which of two cases it is | decision 3: non-owned and nonexistent are one answer |
| Owner of a key you do not hold | links keys to accounts | the representation is only ever built from a row whose `user_id` is the caller's |
| `id` enumeration | ids are a serial, so every id in use can be walked | decision 2: an id you do not own is a `404` whether or not it exists |
| Shared caches | the `200` body carries the key itself, so a stored or replayed response is a credential handed to the next reader; a weak `ETag` on it is also an oracle | `Cache-Control: no-store` and `Vary: Authorization`; no validator on this path, which 013 must not add |
| Access logs and proxies | **the URI is no longer a vector here** — it carries only the id, which says nothing about whom it belongs to | decision 1; the exposure that remains is the response body, which `no-store` actually governs |
| Error bodies | a message naming the owner, or the reason the lookup failed, distinguishes the two cases | the standard error body and nothing more |
| Local client storage | a client storing the whole `200` is storing a credential | out of scope for the server, but the README should say the body carries a secret and must not be logged by the client |

## Out of scope

- `DELETE /auth_keys/{id}`. Addressing the key by its id **unblocks** it —
  revoking a key you are not currently presenting is the lost-or-leaked case,
  and it could never be expressed while the URI named the key value — but it is
  still a capability decision rather than a naming one, and 012 has to land
  first so its `405` is the honest answer for every other method. Candidate for
  its own ticket.
- Listing a user's keys (`GET /auth_keys`). Still 009's out of scope; a client
  holds its own ids and can ask about each in turn.
- Key lifetime, expiry policy, refresh or rotation. Unchanged from 009: the
  30-day default stays, and this ticket only makes it visible.
- Revocation audit, "last used" tracking, or a `created_at` column. The table has
  no creation timestamp and adding one is not needed to report expiry.
- Redacting anything from request logs. No access logging exists today; adding
  it with redaction is a separate change, and the id-based URI means there is
  nothing to redact here.
- Any change to `401` on Bearer-protected endpoints, or to the expiration check
  in `auth_key_to_user`.

## Acceptance criteria

- [ ] `GET /auth_keys/{id}` answers `200` with the representation to the user who
      owns the key, including after that key has expired.
- [ ] `expiration` is RFC 3339 with an explicit offset, and matches the value the
      database holds.
- [ ] The representation carries the key value, `id`, `username` and
      `expiration`.
- [ ] A user naming another user's key gets exactly the same status and body
      shape as a user naming an id that does not exist.
- [ ] A request with no usable credentials is `401` with the bearer challenge and
      says nothing about the named id.
- [ ] A caller holding one valid key can describe another of their own keys which
      has expired, expiry included.
- [ ] A caller whose only key has expired is `401`, which is the accepted
      limitation of decision 4 rather than an oversight.
- [ ] An id which is not a number is `404`, matching how a non-numeric task id is
      answered.
- [ ] The response carries `Cache-Control: no-store` and `Vary: Authorization`,
      and carries no `ETag`.
- [ ] `POST /auth_keys/{id}` and `DELETE /auth_keys/{id}` answer `405` with
      `Allow: GET`.
- [ ] `auth_keys.key` has a unique index and a duplicate insert is refused.
- [ ] `auth_keys.expiration` is `timestamptz`, and `auth_key_to_user` still
      refuses an expired key exactly as it did before the migration.
- [ ] No response anywhere in the API carries an auth key in a URI, a query
      string, or a request body other than the `201` from `POST /auth_keys`.
- [ ] `README.md` documents the endpoint, states that a key is only ever
      described to the user who owns it, and says the representation carries a
      secret.
- [ ] `http_requests/` gains a request for it.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

**Why this depends on 012.** Registering `/auth_keys/{id}` makes actix answer
`405` for every other method, but it answers it *without an `Allow` header*:

```
$ curl -i -X DELETE "…/auth_keys/42" -H "Authorization: Bearer $KEY"
HTTP/1.1 405 Method Not Allowed
content-length: 0
```

RFC 9110 §15.5.6 requires `Allow` on a `405`, so that response is a second
instance of exactly the bug 012 exists to fix. Depending on 012 means this path
inherits the one correct mechanism instead of shipping a headerless `405` or a
hand-rolled second one.

**If you prefer `403`,** decision 3 flips in one line: answer both
someone-else's-key and no-such-id with `403` and the `insufficient_scope`
challenge, as the task endpoints do. The leak property is identical either way —
both statuses give an attacker nothing — so this is a consistency preference, not
a security one. The argument for `403` is that one rule for "hidden resource"
across the whole API reads better in the README and in 015; the argument for
`404` is that it asserts nothing the server is refusing to state.

**A `404` here is cacheable by default** (§15.5.5). It leaks nothing in either
branch, so that is harmless, but 013 should know this path opts out of caching
by policy anyway.

**The id is not a secret and does not need to be one.** It is enumerable, in the
same way task ids are, and the owner check is what makes it safe. Treating it as
opaque would only add obscurity; what protects it is decision 2.
