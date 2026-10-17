# 002: Send a `WWW-Authenticate` challenge with every 401

**Phase:** media types, headers, and status codes (before the redesign)
**Depends on:** none (do 001 first — both touch the same response code)
**Files:** `src/web_api.rs`

## Goal

Every `401 Unauthorized` this server sends must carry a `WWW-Authenticate`
header naming a scheme the client can retry with, so the Bearer protection
space is discoverable and the response is spec-compliant.

## Background

RFC 9110 §15.5.2:

> The 401 (Unauthorized) status code indicates that the request has not been
> applied because it lacks valid authentication credentials for the target
> resource. The server generating a 401 response MUST send a
> WWW-Authenticate header field (Section 11.6.1) containing at least one
> challenge applicable to the target resource.

`/login` already does this: `LOGIN_BASIC_CHALLENGE` (`web_api.rs:89`) is
attached by `with_basic_challenge` (`web_api.rs:144-152`) on every 401 the
login handler produces. Nothing else does.

Three code paths send a bare 401 with no challenge:

- `unauthorized_error` (`web_api.rs:128-136`) — missing or malformed
  `Authorization` header on every Bearer-protected endpoint.
- `ApplicationError::InvalidAuthKey` → 401 (`web_api.rs:49-52`) — a key that is
  unknown or expired.
- `ApplicationError::Unauthorized` → 401 (`web_api.rs:65-68`) — becomes 403
  under ticket 003.

Verified against a running server:

```
$ curl -i http://127.0.0.1:8123/whoami
HTTP/1.1 401 Unauthorized
content-length: 53
date: …
```

No `WWW-Authenticate`. A client that has lost its key has no way to learn that
this API uses `Bearer` at all.

## In scope

- Add a bearer challenge next to `LOGIN_BASIC_CHALLENGE`, e.g.
  `const BEARER_CHALLENGE: &str = "Bearer realm=\"todo\"";` — one realm for the
  whole API, matching the Basic realm.
- Attach it in `unauthorized_error` and on the 401s produced by
  `ApplicationError::into_http_response`.
- Consider RFC 6750 §3 error codes, which tell the client what to do next:
  - `error="invalid_token"` for a presented-but-rejected key
    (`InvalidAuthKey`),
  - `error="invalid_request"` for a missing or malformed header.
  Only one challenge may be sent per header field, so pick the single most
  informative value per path rather than concatenating.
- Make sure the Basic challenge does not leak onto non-login endpoints and the
  Bearer challenge does not leak onto `/login`. `with_basic_challenge` uses
  `headers_mut().insert`, which replaces, so ordering alone is not enough to
  reason about — make the choice explicit at the call site (a helper that takes
  the challenge, or a `login`-only wrapper that asserts it).

## Out of scope

- Changing 401 to 403 where the caller is authenticated but not permitted
  (ticket 003).
- Advertising the bearer scheme via `WWW-Authenticate` on *successful*
  responses or via `OPTIONS`.
- Adding `error="insufficient_scope"` to 403 responses; that lands with
  ticket 003.

## Acceptance criteria

- [ ] Every 401 the server can emit — from a handler or from an extractor —
      carries a `WWW-Authenticate` header.
- [ ] `/login` 401s carry exactly the Basic challenge
      (`Basic realm="todo", charset="UTF-8"`), and no other endpoint's 401
      carries it.
- [ ] Bearer endpoints carry a bearer challenge naming the realm, with an
      RFC 6750 error code where one applies.
- [ ] Existing challenge tests still pass unchanged:
      `login_requires_basic_authorization_header`,
      `refused_credentials_are_challenged_too`.
- [ ] New test: a request to a Bearer endpoint with no `Authorization` header,
      and one with a valid-looking but unknown key, both get the bearer
      challenge.
- [ ] Extract probe route tests updated: `missing_authorization_header_is_rejected`,
      `malformed_authorization_header_is_rejected` assert the challenge is
      present.
- [ ] `README.md`: the endpoint sections that already document the Basic
      challenge on `/login` gain a matching note for the Bearer endpoints.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

Do not log the header value while debugging this; the existing comment at
`web_api.rs:129` is right that it may hold a valid key.
