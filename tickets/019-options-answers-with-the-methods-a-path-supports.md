# 019: `OPTIONS` answers with the methods a path supports

**Phase:** after the redesign — it closes a gap ticket 012 found and could not take
**Depends on:** 012
**Files:** `src/web_api.rs`, `src/lib.rs`, `README.md`, `http_requests/`

## Goal

A request for `OPTIONS` on a path the server knows is answered with the
methods that path supports, in an `Allow` header, rather than with a `405`.
Today it is answered `405 Method Not Allowed`.

## Background

- RFC 9110 §9.3.7 describes `OPTIONS` as the request for the communication
  options available on a target:

  > The OPTIONS method requests information about the communication options
  > available on the request-target or on the server. […] A server MUST
  > generate an Allow header field in a successful (2xx) response to an
  > OPTIONS request, containing a list of the target resource's currently
  > supported methods.

  The same section says a server which does not support `OPTIONS` may ignore
  the request, so the `405` ticket 012 sends is not wrong — it is just a
  refusal to answer a question the API can answer.
- Ticket 012's mechanism answers every method a path does not serve with a
  `405` and an `Allow` header naming the methods it does serve. That `Allow`
  header is already the answer `OPTIONS` is asking for; what is missing is a
  way to ask for it without making a request the server has to refuse.
- A client which wants to know what a path supports currently has to guess: send
  a request it expects to fail and read the `Allow` header of the failure. That
  works, but it is a request which is refused by design, and a client which
  treats a `405` as an error has to be taught to treat it as an answer.

## In scope

- `OPTIONS` on every path the server knows, answered `204 No Content` with an
  `Allow` header naming the methods that path supports.
- The `Allow` header of the `405` on those paths, which lists the methods a
  path supports, so it has to list `OPTIONS` too once `OPTIONS` is served.
  RFC 9110 §15.5.6 asks for the list of the methods the resource supports, and
  `OPTIONS` is one of them from then on. This is a visible change to the
  responses of the paths it touches: `Allow: GET, POST` becomes
  `Allow: GET, POST, OPTIONS`.
- A test which says an `OPTIONS` request is answered with the methods the path
  supports.

## Decisions

1. **`OPTIONS` is answered, not refused.** The API can say what a path
   supports — ticket 012 made that answer correct — so refusing to say it
   answers the question with a refusal.
2. **`204 No Content`, with no body.** RFC 9110 §9.3.7 asks for the header
   field and nothing else; the answer is the `Allow` header, and a body would
   have to be invented for its sake.
3. **The `Allow` header of the `OPTIONS` answer names the same methods as the
   `Allow` header of the `405` on that path.** They are the same list, and a
   client which reads one has read the other.
4. **An `OPTIONS` request to a path the server does not know is still a
   `404`.** The path names no resource, so there is no list of methods to
   give.

## Out of scope

- The server-wide `OPTIONS *` of RFC 9110 §9.3.7, which asks about the server
  rather than about a resource.
- `TRACE`.
- `HEAD`, which is ticket 018.
- Content negotiation, and the `406` a representation the client will not
  accept would bring. The API serves JSON only.

## Acceptance criteria

- [ ] `OPTIONS /tasks` answers `204 No Content` with an `Allow` header naming
      the methods `/tasks` supports.
- [ ] `OPTIONS` is answered on every path the server knows.
- [ ] The `Allow` header of the `OPTIONS` answer and of the `405` on the same
      path name the same methods.
- [ ] `OPTIONS` on a path the server does not know is still a `404`.
- [ ] A test asserts the `Allow` header of an `OPTIONS` answer against the
      methods the path supports.
- [ ] `README.md` documents the `OPTIONS` answer, and `http_requests/` gains a
      request for it.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

**Why this depends on 012.** The `Allow` header is what says which methods a
path supports, and 012 is what makes it honest. Answering `OPTIONS` means
answering with that list, so it lands after 012 rather than beside it: an
`OPTIONS` answer written before the `Allow` header existed would have been an
answer the API could not keep true.
