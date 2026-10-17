# 003: Use 403 for authorization failures, not 401

**Phase:** media types, headers, and status codes (before the redesign)
**Depends on:** 002 (both rewrite the 401 paths)
**Files:** `src/web_api.rs`, `src/core.rs`

## Goal

Distinguish "you are not authenticated" (401) from "you are authenticated but
not allowed" (403), without reintroducing an oracle that reveals which task ids
exist.

## Background

RFC 9110 §15.5.4:

> The 403 (Forbidden) status code indicates that the server understood the
> request but refuses to fulfill it. […] If authentication credentials were
> provided in the request, the server considers them insufficient to grant
> access.

The API collapses two different failures into 401:

- **No/invalid credentials** → 401 is correct. (`unauthorized_error`,
  `ApplicationError::InvalidAuthKey`.)
- **Valid credentials, insufficient permission** → 401 is wrong. This covers
  reading a task you have no share on (`web_api.rs:373-381`), updating a task
  with only a `read` share, sharing or listing shares on someone else's task,
  and modifying a task that does not exist.

A client cannot act on the difference: 401 says "authenticate again", so a
client with a perfectly good key will keep re-authenticating against a request
that can never succeed.

The intent behind the current behavior is sound. `get_task_by_id` and
`update_task_by_id` deliberately return the same status for "task does not
exist" and "task exists but is not yours" (`web_api.rs:382-391`,
`web_api.rs:832-841`; `require_task_owner` in `core.rs:700-718`), and RFC 9110
§15.5.4 explicitly allows that shape:

> An origin server that wishes to "hide" the current existence of a forbidden
> target resource MAY instead respond with a status code of 404.

So the rule to keep is "same status for both", not "use 401".

## In scope

- Split `ApplicationError` (`core.rs:10-19`) so the two cases are distinct at
  the type level, e.g. keep `Unauthorized` for missing/invalid credentials and
  add `Forbidden` for authenticated-but-refused. Update the producers:
  - `update_task` (`core.rs:662-669`) returns the new variant.
  - `require_task_owner` (`core.rs:700-718`) returns the new variant.
- Map the new variant to 403 in `ApplicationError::into_http_response`
  (`web_api.rs:37-72`).
- `get_task_by_id`'s inline 401s (`web_api.rs:373-391`) become 403, keeping the
  existing-task and missing-task cases identical.
- Per RFC 6750 §3, a 403 should carry
  `WWW-Authenticate: Bearer error="insufficient_scope"`; wire that into
  ticket 002's bearer-challenge helper rather than inventing a third mechanism.
- A missing/invalid auth key keeps returning 401 with the bearer challenge from
  ticket 002.

## Out of scope

- Returning 404 for both cases instead of 403. It hides existence more
  aggressively, but 403 plus an identical response for both cases already
  satisfies the oracle rule and keeps "your key is fine, your permissions are
  not" diagnosable. Revisit only if a threat model demands it.
- Any change to *which* callers have access. This ticket changes status codes
  only.
- Deciding the status for an unknown target username in `share_task`
  (`/unshare_task`) — that is a real resource lookup and stays 404.

## Acceptance criteria

- [ ] Missing, malformed, or unknown auth key → 401 with a bearer challenge.
- [ ] Authenticated but not permitted → 403 with `error="insufficient_scope"`.
- [ ] "Task does not exist" and "task exists but is not yours" return the same
      status (403) and the same body shape for `GET /task_by_id`,
      `PATCH /task_by_id`, `POST /share_task`, `GET /task_shares`, and
      `DELETE /unshare_task`.
- [ ] `ApplicationError` no longer uses one variant for both failure kinds.
- [ ] Tests updated to the corrected statuses — at minimum:
      `cannot_get_different_users_task`, `cannot_get_nonexistent_task`,
      `cannot_update_nonexistent_task`, `no_share_means_401_and_no_listing_entry`,
      `non_owner_cannot_share_unshare_or_list`,
      `missing_task_returns_same_401_as_not_owned`,
      `read_only_sharee_cannot_update_task`, `unrelated_user_cannot_update_task`,
      `read_write_sharee_cannot_manage_shares`,
      `cannot_add_task_without_valid_auth_key` (this one stays 401 — a bad key
      really is an authentication failure; do not change it).
- [ ] Rename tests whose names assert the old status
      (`missing_task_returns_same_401_as_not_owned`,
      `no_share_means_401_and_no_listing_entry`) so the name does not lie.
- [ ] `README.md` documents the 401/403 split.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

Two of these tests deliberately assert the *wrong* status today
(`cannot_get_nonexistent_task` asserts 401; `refused_credentials_are_challenged_too`
asserts 404 for a login with an unknown user). Fixing those means changing the
test's expectation, not weakening it.
