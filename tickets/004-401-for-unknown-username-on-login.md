# 004: Answer `401`, not `404`, for an unknown username at `/login`

**Phase:** media types, headers, and status codes (before the redesign)
**Depends on:** none
**Files:** `src/web_api.rs`, `README.md`

## Goal

A refused authentication attempt is reported as a refused authentication
attempt, whether or not the named user exists.

## Background

`POST /login` looks the user up by name first (`web_api.rs:601-606`,
`get_user_by_name` in `core.rs:599-629`) and lets the result fall through to
the generic error mapper, where a diesel `NotFound` becomes `404 Not Found`
(`web_api.rs:43-45`).

Verified against a running server:

```
$ curl -i -X POST …/login -u nosuchuser:whatever
HTTP/1.1 404 Not Found

$ curl -i -X POST …/login -u alice:wrongpassword
HTTP/1.1 401 Unauthorized
www-authenticate: Basic realm="todo", charset="UTF-8"
```

The endpoint exists. The credentials were refused. RFC 9110 §15.5.2 describes
this case — *"the request has not been applied because it lacks valid
authentication credentials for the target resource"* — and §15.5.5 describes
404, *"the origin server did not find a current representation for the target
resource or is not willing to disclose that one exists"*. Neither the endpoint
nor the protection space is missing here, so 404 is the wrong status, and it
makes the endpoint's contract depend on an implementation detail: a lookup
miss in `core.rs` decides the status a caller sees.

It also leaves the endpoint inconsistent with itself: a wrong password answers
401 with a challenge that tells the client how to retry, while a wrong username
answers 404 with no indication of what went wrong.

## Scope note

This is a status-code fix, not a hardening fix. **Account enumeration is an
accepted trade-off for this API** — `/register_account` already returns `409
Conflict` for a taken username, so existence is observable regardless, and
there are no clients whose accounts need protecting from it. Consequently this
ticket deliberately does **not**:

- verify the password against a dummy Argon2 hash on the unknown-user path to
  equalize response timing;
- change the `409` on duplicate registration;
- delay or obscure the miss in any other way.

If that trade-off is ever revisited, the timing work belongs here and the
`409` decision in ticket 010.

## In scope

- Map a missing user in the login path to the same `401` + Basic challenge as a
  wrong password, with the same body shape.
- Keep the challenge handling in ticket 002 correct for this endpoint: the
  Basic challenge belongs here and nowhere else.
- Log the failed lookup at the existing level. The submitted username may now
  be logged as it would be for any other failed request — it is already
  present in the request — but keep it at `TRACE` to match how `login`
  currently logs.

## Out of scope

- Timing equalization, rate limiting, lockout, or account-status handling.
- Any status change for a task or user id that does not exist. Those are real
  resource lookups and `404` is correct for them.
- `Username::new` returning `400` for an empty or whitespace-only username
  (`web_api.rs:591-597`). That is a validation error about the header syntax,
  not an authentication verdict, so it stays `400`.

## Acceptance criteria

- [ ] `POST /login` with an unknown username returns `401` with
      `WWW-Authenticate: Basic realm="todo", charset="UTF-8"`.
- [ ] Unknown username and wrong password return the same status, the same
      headers, and the same body shape (only `request_id` differs).
- [ ] `login_requires_basic_authorization_header` still passes unchanged.
- [ ] `refused_credentials_are_challenged_too` updated: its final block
      currently asserts `404` for an unknown username and asserts that no
      challenge is present. Both assertions become `401` + challenge present.
- [ ] `invalid_password_fails` still passes.
- [ ] New test: an unknown username and a wrong password produce responses
      differing only in `request_id`.
- [ ] `README.md` `/login` section states that a refused login is `401`
      whether or not the username exists.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.
