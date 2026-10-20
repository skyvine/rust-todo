# 017: A share is readable at the URI its creation names

**Phase:** after the redesign — it closes a gap ticket 011 found and could not take
**Depends on:** 008, 011, 012
**Files:** `src/web_api.rs`, `src/core.rs`, `src/lib.rs`, `README.md`, `http_requests/`

## Goal

`PUT /tasks/{id}/shares/{username}` answers `201 Created` with
`Location: /tasks/{id}/shares/{username}`, and nothing is served at that URI.
Serve the share's representation there.

## Background

- Ticket 008 made a share a sub-resource of the task and gave the upsert `PUT`
  semantics, so the path `/tasks/{id}/shares/{username}` is registered for `PUT`
  and `DELETE` but not for `GET`. Actix registers each attribute macro as its
  own resource with a method guard, so a request for that path with any *other*
  method matches no resource and falls through to the default 404 handler
  (the same mechanism ticket 012 exists to correct). A client which follows the
  `Location` a creation hands it is therefore told the share does not exist.
- The share is not unreachable in any other way: `GET /tasks/{id}/shares` serves
  the collection, and every entry in it is a share. What is missing is the
  share at its own URI — the one URI the API names a share by.
- RFC 9110 §15.3.2 identifies the primary resource created by a request by the
  `Location` header when one is received, and only falls back to the target URI
  when it is not. A `Location` which answers 404 is therefore worse than no
  `Location` at all: it tells a client that the resource it was just handed does
  not exist.
- Ticket 011 states the rule — every `201` names the resource it created — and
  names this as one of the two gaps it could not close itself: the other is
  `/auth_keys/{id}`, which is ticket 016.

## In scope

- `GET /tasks/{id}/shares/{username}`, answering `200` with the share's
  representation in the same shape the collection uses: `id`, `username` and
  `permission`.
- A core function which reads one share of one task, next to
  `add_share_for_task`, `remove_share` and `list_shares_for_task` in
  `src/core.rs`.
- The `Location` of a created share answering successfully, and a test which
  follows it.

## Decisions

1. **Only the owner of the task may read a share.** The collection is
   owner-only, and so are `PUT` and `DELETE` on a share, so a `GET` which
   answered a grantee would be the one method on a share which is not the
   owner's business. It would also answer a question nobody asked: the grantee
   knows what permission they hold, because the API told them when it granted
   it and because the task they can read is the task the share is on.
2. **A task the caller does not own and a task which does not exist are the
   same answer**, which is the rule the whole API already follows for tasks
   (`require_task_owner` in `src/core.rs` answers `Forbidden` for both). A
   share on a task the caller cannot see is not reported, so a share cannot be
   used to find out which task ids exist.
3. **A share which is not there is a `404`.** The caller owns the task, so they
   can list its shares and see for themselves that the share is not among them;
   hiding it would protect nothing. This is the same shape as the `404` a
   `DELETE` on a share which is not there already answers — that one is a `204`,
   because `DELETE` is idempotent and the state asked for is the state reached.
4. **A username no account has is a `404`.** Usernames and user ids are
   public, so this is a real miss rather than something to hide, and it is what
   `PUT` and `DELETE` on a share already answer.
5. **The representation is the one the collection uses.** `share_body` in
   `src/web_api.rs` builds it, and both the listing and the `PUT` already build
   their entries with it, so one share has one representation whichever way a
   client reads it.
6. **`GET` joins `PUT` and `DELETE` on the path**, so the path's `Allow` is
   `GET, PUT, DELETE` once ticket 012's mechanism is in place.

## Out of scope

- `PATCH` on a share. The URI fully specifies the share and the body says what
  it grants, so a permission is replaced by `PUT` and there is nothing partial
  to express.
- A listing of the shares a user holds across tasks. The API offers no listing
  of tasks by share either, and a client which needs one can walk its tasks.
- Changing who may read a share, or the rule which hides a task. Both are the
  API's existing decisions, not this ticket's to revisit.
- `DELETE /tasks/{id}/shares/{username}`'s status, and the `PUT`'s. Both are
  already what they should be.
- `/auth_keys/{id}`, which is ticket 016.

## Acceptance criteria

- [ ] `GET /tasks/{id}/shares/{username}` answers `200` with the share's `id`,
      `username` and `permission`, in the same shape the collection uses.
- [ ] The `Location` of a created share answers `200` with that share, and a
      test follows the `Location` a creation hands it.
- [ ] A share which is not there is a `404`, for a username an account has.
- [ ] A username no account has is a `404`.
- [ ] A task the caller does not own is answered exactly as a task which does
      not exist: the same status and the same body shape.
- [ ] A request with no usable credentials is a `401` with the bearer
      challenge.
- [ ] `PUT` and `DELETE` on the same path keep working, and the path's `Allow`
      lists `GET`, `PUT` and `DELETE`.
- [ ] `README.md` documents the endpoint, and the note which says that nothing
      is served at the URI of a single share is replaced by the endpoint.
- [ ] `http_requests/` gains a request for it.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

**Why this depends on 012.** Registering `GET` on a path which already carries
`PUT` and `DELETE` adds a method to it, and 012's mechanism is what keeps the
`Allow` header honest about which methods a path supports. Landing this after
012 means the path inherits the one correct mechanism instead of a second,
hand-rolled one.

**A `404` here is cacheable by default** (§15.5.5). It leaks nothing, so that
is harmless, but 013 should know this path opts out of caching by policy
anyway.
