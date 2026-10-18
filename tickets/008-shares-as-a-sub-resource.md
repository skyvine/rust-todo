# 008: Shares as a sub-resource of a task

**Phase:** the redesign
**Depends on:** 005, 006
**Files:** `src/web_api.rs`, `src/core.rs`, `README.md`, `http_requests/`

## Goal

A share is a sub-resource with its own URI. This removes the DELETE-with-a-body
violation, gives share creation `PUT` semantics, and makes a share's
representation a thing a client can read, change, and delete by URI.

## Background

Three endpoints implement sharing today, and all three name an action:

- `POST /share_task` — body `{id, username, permission}`. Creates the share or
  replaces the permission on an existing one (`add_share_for_task`,
  `core.rs:722-763`, an `ON CONFLICT … DO UPDATE` upsert).
- `DELETE /unshare_task` — body `{id, username}` (`web_api.rs:863-896`).
- `GET /task_shares?id=N` — list the shares on a task.

Two distinct problems:

**The DELETE carries a request body.** RFC 9110 §9.3.5:

> content received in a DELETE request has no generally defined semantics,
> cannot alter the meaning or target of the request, and might lead some
> implementations to reject the request and close the connection because of its
> potential as a request smuggling attack.

Verified: `DELETE /unshare_task` with a body succeeds, and the same request
without a body returns `400`. The operation only works because the body is
read, so any intermediary that strips DELETE bodies breaks it.

**The upsert is a PUT wearing a POST.** `POST` is for creating a resource that
did not exist; this operation assigns a permission to a fully-specified URI,
creates or replaces in one step, and is idempotent. That is `PUT`. It also
answers `200` even when it creates (`web_api.rs:739`), so a client cannot tell
the two cases apart, and sends no `Location` for the new share.

## In scope

- `GET /tasks/{id}/shares` — list the shares on a task owned by the caller.
  Same permission checks and the same existence-oracle rule as today.
- `PUT /tasks/{id}/shares/{username}` — body `{"permission": "read" |
  "read_write"}`. Creates or replaces the share at that URI.
  - `201 Created` with `Location: /tasks/{id}/shares/{username}` and the share's
    representation when it created the share; `200 OK` with the share when it
    replaced an existing permission.
  - This requires `add_share_for_task` to report which case happened.
    `Result<(), ApplicationError>` cannot tell the caller, and
    `ON CONFLICT DO UPDATE` returns the updated row in both cases. Practical
    route: attempt `ON CONFLICT DO NOTHING … RETURNING id` first — a returned
    row means it was created (`201`), no row means it already existed and
    should then be updated (`200`) — or read the existing row before writing
    and accept the small race in exchange for a clearer query.
  - Keep `Username::new` validation and the 400 for an invalid permission
    (`web_api.rs:712-729`).
- `DELETE /tasks/{id}/shares/{username}` — **no request body.** The target is
  fully specified by the URI.
  - When the share does not exist, return 204 to keep DELETE idempotent.
- Remove `ShareTaskPayload` and `UnshareTaskPayload` (`web_api.rs:698-703`,
  `854-858`).
- The envelope stays as-is here; ticket 014 removes it after all the URIs have
  moved. Each share then returns as a share object (at least `username` and
  `permission`; consider adding the share's `id`, which `TaskShare` already
  carries and no endpoint exposes).

## Out of scope

- Who may share: today only the owner (`require_task_owner`), unchanged.
  Letting a `read_write` grantee share onward is a policy decision, not a REST
  one.
- A share with a user who has no account, group shares, or an "anyone with the
  link" share.
- `PATCH` on a share. `PUT` covers the whole representation; there is no
  partial update to express.

## Acceptance criteria

- [x] `GET`, `PUT`, and `DELETE` under `/tasks/{id}/shares` exist;
      `/share_task`, `/task_shares`, and `/unshare_task` are gone.
- [x] No endpoint reads a request body on a `DELETE`.
- [x] `PUT` returns `201` + `Location` when it creates a share and `200` when
      it replaces an existing permission; the core function reports which.
- [x] Re-sharing with the same permission is still safe to repeat.
- [x] `DELETE` of an absent share returns the documented status, consistently.
- [x] The existence-oracle rule still holds: a task that does not exist and a
      task owned by someone else return the same status and body shape on all
      three verbs.
- [x] Tests updated: the `share_task`, `unshare_task`, and `get_task_shares`
      helpers, plus `owner_can_share_and_see_share_listed`,
      `resharing_changes_the_permission` (extend it to assert the 201/200
      distinction), `non_owner_cannot_share_unshare_or_list`,
      `unsharing_removes_the_row`, `unknown_target_username_gives_404`,
      `missing_task_returns_same_401_as_not_owned`,
      `read_write_sharee_cannot_manage_shares`,
      `cannot_list_shares_without_a_valid_id_query_parameter` (repurpose or
      delete — there is no query string left).
- [x] `http_requests/post_share_task.http` and
      `http_requests/delete_unshare_task.http` are rewritten; the DELETE
      request has no body.
- [x] `README.md` documents the three share endpoints, including the
      201-vs-200 rule.
- [x] `cargo clippy` passes with no new warnings.
- [x] `cargo test` passes.
- [x] `cargo fmt --check` passes.

## Notes

`unknown_target_username_gives_404` is the one place where a 404 is correct and
should stay one: an unknown username is a real resource lookup on a resource
whose existence the API already treats as knowable (`GET /users/{id}`), not an
authorization decision about the caller's access to a task.
