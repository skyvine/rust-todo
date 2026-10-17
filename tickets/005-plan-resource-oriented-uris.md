# 005: Plan the resource-oriented URI redesign

**Phase:** plan
**Depends on:** none — read this before 006–010
**Files:** none (design only, or `README.md` if the decisions are recorded there)

## Goal

Agree on one URI scheme for every resource in the API, decide how the existing
URIs are retired, and write down the decisions so tickets 006–010 can be
implemented without re-litigating them.

## Background

Fielding's architectural style treats the URI as the identifier of a resource
(constraints 3 and 4). The current design departs from that in specific,
enumerable ways:

| Current | Problem |
|---|---|
| `POST /add_task` | verb in the URI; one task resource under three URIs |
| `GET /task_by_id?id=N`, `PATCH /task_by_id` with `id` in the body | id not in the URI |
| `GET /all_tasks`, `GET /incomplete_tasks` | one collection, two URIs |
| `POST /share_task`, `DELETE /unshare_task` (with a body) | verbs in the URI; DELETE carries a body |
| `POST /login`, `DELETE /logout` | verbs in the URI; the resource is an auth key |
| `GET /whoami`, `GET /user_by_id?id=N`, `POST /register_account` | verbs in the URI; id not in the URI |

The recent commit history (`71dcf0c`…`431acd1`) already moved authentication
off request bodies and onto headers, and `0b55125` moved a POST to PATCH — the
direction of travel is established. This plan finishes it. The conformance
tickets that carry over (001, 002, 003, 004) are independent and can land
first.

## Proposed URI scheme

| Method | URI | Replaces | Ticket |
|---|---|---|---|
| `POST` | `/users` | `POST /register_account` | 010 |
| `GET` | `/users/me` | `GET /whoami` | 010 |
| `GET` | `/users/{id}` | `GET /user_by_id?id=N` | 010 |
| `POST` | `/auth_keys` | `POST /login` | 009 |
| `DELETE` | `/auth_keys` | `DELETE /logout` | 009 |
| `DELETE` | `/auth_keys/{key}` | — (new: revoke a specific key) | 009 |
| `GET` | `/tasks` | `GET /all_tasks` | 007 |
| `GET` | `/tasks?completed=false` | `GET /incomplete_tasks` | 007 |
| `POST` | `/tasks` | `POST /add_task` | 006 |
| `GET` | `/tasks/{id}` | `GET /task_by_id?id=N` | 006 |
| `PATCH` | `/tasks/{id}` | `PATCH /task_by_id` (id in body) | 006 |
| `GET` | `/tasks/{id}/shares` | `GET /task_shares?id=N` | 008 |
| `PUT` | `/tasks/{id}/shares/{username}` | `POST /share_task` | 008 |
| `DELETE` | `/tasks/{id}/shares/{username}` | `DELETE /unshare_task` (with a body) | 008 |
| `GET` | `/health` | `GET /is_alive` | 010 |

`PATCH /tasks/{id}` keeps its method — partial update of a task is what PATCH
is for — and only its URI changes. `PUT /tasks/{id}/shares/{username}` is new:
sharing with a named user at a fully-specified URI is an idempotent
assignment, which is what PUT means. It returns `201` with `Location` when the
share is created and `200` when an existing permission is replaced.

## Decisions Made

1. **Use plural collection nouns.** `/tasks` over `/task`
2. **Path parameter for shares: username.** Usernames are what
   clients have when they share (the API accepts them today), so
   `.../shares/{username}` avoids a lookup round-trip. Ids are stable under a
   hypothetical rename, and usernames are not. Use usernames, and accept
   that a rename would need to touch share rows or resolve the name at request
   time — that is already true of the current implementation.
3. **Rollout: hard cut.** There are no clients, so the old URIs are deleted in
   the same release that introduces the new ones. No `/v2` prefix, no
   transition window, no compatibility aliases. Permanent aliases are doubly
   wrong here: they would keep two URIs per resource, which is the problem
   being fixed, and they would need maintaining forever.
4. **Whether the transition also drops the response envelope**, so a URI
   returns its resource directly. Previously staged as its own ticket on the
   grounds that two rounds of shape changes are disruptive; with no clients
   there is nothing to disrupt, so it is folded into ticket 014.
5. **Content types.** Whether to keep bare `application/json` everywhere or
   introduce a hypermedia format. Keep JSON; hypermedia is a larger project
   and is not required to fix the URI design.

## Out of scope

- Implementing any of the above. This ticket produces decisions, and the
  implementation lands in 006–010.
- The conformance tickets. Those are independent of this plan and the ones that
  survive the redesign (001, 002, 003, 004) can land before it.

## Acceptance criteria

- [X] Every decision above is answered and written down, in this ticket or in
      `README.md` where a caller will read it.
- [X] The URI table is final, including whether `/tasks/{id}` supports DELETE.
- [X] The hard-cut rollout is recorded, with the note that there are no
      clients to transition.
- [X] Nothing in the conformance tickets depends on the outcome of this one.

## Notes

The audit behind this plan was done against a running server, not just the
source, so the URI table reflects observed behavior. Two details worth
preserving while redesigning:

- Responses currently carry no `Content-Type` (ticket 001, which lands before
  this). `201` carries no `Location` — ticket 011 is sequenced after the
  redesign for exactly that reason, so the new creation endpoints point
  `Location` at the final URIs rather than at transitional ones.
- The `401` responses that stand in for "forbidden" become `403` under ticket
  003, and the task existence-oracle rule — missing task and not-your-task
  return the same status — must survive the redesign unchanged.
