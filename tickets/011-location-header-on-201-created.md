# 011: Every `201 Created` identifies the resource it created

**Phase:** media types, headers, and status codes (after the redesign)
**Depends on:** 005, 006, 010
**Files:** `src/web_api.rs`, `src/core.rs`, `README.md`

## Status

The work was split across the redesign. Tickets 006, 008, 009 and 010 each
implement an endpoint which creates a resource, and each of them landed the
`Location` header and the created resource in the body, because a creation
endpoint landed knowingly wrong would have been strange. Ticket 010 also
landed the `add_user` core change, since `POST /users` cannot answer without
it.

What is left is what no single endpoint could own: the rule stated once, the
two `Location` values which name a URI nothing is served at, and a test for the
one thing every endpoint could have got wrong. The criteria below are the
ticket's, narrowed where the redesign changed what they could mean, and
checked where the work is done.

The two unserved `Location` URIs are not left undone: the auth key's is ticket
016, and the share's is ticket 017.

## Goal

A client that creates a resource learns the URI of what it created, from the
response, without scraping an id out of a body field.

## Why this comes after the redesign

The obvious first move is to add `Location: /task_by_id?id={id}` and be done.
That value would be obsolete as soon as ticket 006 lands, because
`/task_by_id` is itself replaced by `/tasks/{id}`. With no clients to
transition, the useful order is: land the URIs, then add `Location` pointing at
the final ones.

## Background

RFC 9110 §15.3.2:

> The primary resource created by the request is identified by either a
> Location header field in the response or, if no Location header field is
> received, by the target URI.

Neither creation endpoint identified its result before the redesign:

- `POST /add_task` returned `201 Created` with
  `{"request_id": …, "task_id": 2424}` (`web_api.rs:562-566`). The target URI
  is `/add_task`, which is not the new task, so the fallback clause does not
  apply and the caller is left holding a bare integer.
- `POST /register_account` returned `201 Created` with **an empty body**
  (`HttpResponse::Created().finish()`, `web_api.rs:681`) — not even the id.

`add_user` (`core.rs:359-392`) discarded the inserted row, so the handler could
not know the id at all without a core change.

## The rule

Every `201` in the API carries a `Location` naming the resource it created, and
carries that resource in its body:

| Endpoint | `Location` | Body |
|---|---|---|
| `POST /tasks` | `/tasks/{id}` | the task |
| `POST /users` | `/users/{id}` | `{id, username}` |
| `POST /auth_keys` | `/auth_keys/{id}` | the key itself |
| `PUT /tasks/{id}/shares/{username}` | `/tasks/{id}/shares/{username}` | the share |

A `PUT` on a share which replaces the permission on a share which is already
there creates nothing, so it answers `200 OK` and carries no `Location`: the
status is what tells the two apart.

The `Location` is a relative reference, never the whole URL, and never a
secret. An auth key is named by the id of its row rather than by its value,
because a bearer token in a URL is recorded by every proxy and log which
records a request line (RFC 6750 §5.3).

## In scope

- **The rule**, applied to every `201` in the API after the redesign, and
  stated once in `README.md` rather than repeated per endpoint.
- **`add_user` must report what it created.** It returns the created user (or
  at least its id) instead of `()`, keeping the `UserExists` → `409` mapping
  intact. This is the one piece of genuinely independent work in the ticket,
  and it is what makes `POST /users` able to answer at all.
- **Return the created resource, not just its id.** `POST /tasks` returns the
  task (the `Task` struct already derives `Serialize`, so it is
  `Task::from_json_object`-compatible); `POST /users` returns a struct with
  `id` and `username`. **Never** return the password hash — note that `User`
  derives `Queryable, Selectable, ZeroizeOnDrop` but not `Serialize`, so a
  response struct is required rather than serializing `User`.
- **Remove the id-only fields.** The bare `task_id` key has no reason to exist
  once the representation is present; delete it rather than keeping it as a
  deprecated alias, since there are no clients reading it.
- `Location` values are built from the id the server stored — a test should
  `GET` the `Location` URI and get the resource back.

## Out of scope

- Whether `POST /users` or `POST /register_account`-style creation should be a
  `PUT` at a client-chosen name. Decided in ticket 010.
- `202 Accepted` for asynchronous work; nothing here is asynchronous.
- Hypermedia links beyond `Location` — `Link` headers pointing at the share
  collection would be a natural follow-up, but a plain `Location` is what the
  spec asks for.
- Serving the two URIs which a `Location` names and nothing else serves:
  `/auth_keys/{id}` is ticket 016, and `/tasks/{id}/shares/{username}` is ticket
  017. This ticket narrows the criterion rather than taking either over.

## Acceptance criteria

- [x] Every `201` in the API carries a `Location` naming the resource it
      created.
- [x] The `Location` of a created task and of a created user `GET`s
      successfully and returns the resource it names.
- [ ] The `Location` of a created auth key names a URI which is served —
      ticket 016.
- [ ] The `Location` of a created share names a URI which is served — ticket
      017.
- [x] `POST /tasks` returns the created task; the bare `task_id` key is gone.
- [x] `POST /users` returns a representation containing `id` and `username`,
      and no password hash.
- [x] `add_user` returns the created user and still maps a unique violation to
      `409`.
- [x] No response body anywhere in the API contains a password hash.
- [x] `README.md` documents `Location` once, as a general rule, rather than
      repeating it per endpoint.
- [x] `cargo clippy` passes with no new warnings.
- [x] `cargo test` passes.
- [x] `cargo fmt --check` passes.

## Notes

Ticket 006 and 010 both mention `Location` in their acceptance criteria because
their endpoints are creations and it would be strange to land them knowingly
wrong. If this ticket lands before them, the two should drop that line rather
than duplicate the work — either order is acceptable, but not both
implementations.

**The criterion narrowed, and why.** "Every `201` carries a `Location` that
`GET`s successfully" was written when the API had four creation endpoints and
two served the `Location` URI. It cannot be met without adding
`GET /auth_keys/{id}` — which is the whole subject of ticket 016, and which
carries its own ownership and expiry decisions — or `GET
/tasks/{id}/shares/{username}`, which no ticket had claimed. Rather than take
either over, this ticket states the rule and names the two gaps, and each gap
has a ticket of its own: 016 for the auth key, 017 for the share.

**The two unserved URIs are a real defect, not a subtlety.** A `Location` which
answers `404` is worse than no `Location` at all: RFC 9110 §15.3.2 falls back
to the target URI only when no `Location` is received, so a client which
follows the header is told the resource does not exist, while a client which
ignores it is sent back to the collection. The `README.md` says so plainly
where it documents each of them, rather than letting a client discover it.
