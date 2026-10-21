# 018: `HEAD` is answered wherever `GET` is

**Phase:** after the redesign — it closes a gap ticket 012 found and could not take
**Depends on:** 012
**Files:** `src/web_api.rs`, `src/lib.rs`, `README.md`, `http_requests/`

## Goal

A request for `HEAD` on a path which serves `GET` is answered the way the
`GET` would be, minus the content: the same status, the same headers, and no
body. Today it is answered `405 Method Not Allowed`.

## Background

- RFC 9110 §9.3.2:

  > The HEAD method is identical to GET except that the server MUST NOT send
  > content in the response. The server SHOULD send the same header fields in
  > the response as it would have sent if the request method had been GET.
  > However, a server MAY omit header fields for which a value is determined
  > only while generating the content.

- Actix does not route a `HEAD` request to a `GET` handler. A `GET` route's
  guard matches `GET` and nothing else, so a `HEAD` request matches no route
  at all. Ticket 012's mechanism then answers it with a `405` and an `Allow`
  header naming the methods the path supports.
- That answer is honest — the resource does not implement `HEAD` — but it is
  not what RFC 9110 §9.3.2 describes for a resource which serves `GET`. A
  client which uses `HEAD` to ask whether a resource is there, or to fetch its
  headers without its body, is refused a question it is entitled to ask.
- **A correction to ticket 012.** Ticket 012's notes say that "actix answers
  `HEAD` for any `GET` route by running the handler and dropping the body".
  That is not true of actix-web 4.11, which was checked against a running
  server while 012 was being implemented: `HEAD` on a `GET` path was a `404`
  before 012 and is a `405` after it. What actix-web does do is drop the
  content of the response to a `HEAD` request, so that a handler which answers
  a `HEAD` request sends its headers and no body — which is the second half of
  what RFC 9110 §9.3.2 asks for.

## In scope

- `HEAD` on every path which serves `GET`: `/tasks`, `/tasks/{id}`,
  `/tasks/{id}/shares`, `/users/me`, `/users/{id}` and `/health`.
- The `Allow` header of a `405` on those paths, which lists the methods a path
  supports, so it has to list `HEAD` too once `HEAD` is served. RFC 9110
  §15.5.6 asks for the list of the methods the resource supports, and `HEAD`
  is one of them from then on.
- A test which says a `HEAD` request is answered like the `GET` it stands in
  for.

## Decisions

1. **`HEAD` is served wherever `GET` is.** A resource which can be read can be
   asked about without its content being sent, and RFC 9110 §9.3.2 says the
   two are the same request apart from the content. Serving `HEAD` on some
   paths and not others would leave a client guessing which is which.
2. **The handler runs and the content is dropped.** Actix-web already omits
   the content of a response to a `HEAD` request, so the `GET` handler can
   answer the `HEAD` request unchanged. The alternative — a `HEAD` handler
   which does the work of the `GET` handler without building the body — buys
   the cost of a second handler per path and of keeping the two in step, and
   is worth doing only if the bodies turn out to be expensive.
3. **A `HEAD` request which is refused is refused as the `GET` would be.** A
   `HEAD` on a path which takes an auth key and does not present one is a
   `401` with the bearer challenge, exactly as the `GET` is; the difference is
   only that no content is sent.

## Out of scope

- `HEAD` on a path which does not serve `GET` — `/users`, `/auth_keys`,
  `/tasks/{id}/shares/{username}`. There is no `GET` to stand in for, so
  `HEAD` there stays a `405` naming the methods the path does serve.
- `OPTIONS`, which is ticket 019.
- `TRACE`, and the server-wide `OPTIONS *` of RFC 9110 §9.3.7.

## Acceptance criteria

- [ ] `HEAD /tasks` answers the status the `GET` answers, with the same
      headers and no content.
- [ ] `HEAD` is served on every path which serves `GET`.
- [ ] The `Allow` header of a `405` on such a path lists `HEAD` along with the
      other methods the path serves.
- [ ] `HEAD` on a path which does not serve `GET` is still a `405`, with an
      `Allow` header naming the methods it does serve.
- [ ] A test asserts the status of a `HEAD` request against the status of the
      `GET` it stands in for.
- [ ] `README.md` documents the `HEAD` answer, and `http_requests/` gains a
      request for it.
- [ ] `cargo clippy` passes with no new warnings.
- [ ] `cargo test` passes.
- [ ] `cargo fmt --check` passes.

## Notes

**Why this depends on 012.** The `Allow` header is what says which methods a
path serves, and 012 is what makes it honest. Serving `HEAD` on a path adds a
method to it, so it lands after 012 rather than beside it: a `HEAD` route
added before the `Allow` header existed would have been a `HEAD` the API never
said it had.

**A `405` on a `HEAD` request is not a `404`.** A path which serves `GET` and
not `HEAD` is a path which exists, so the `405` is the honest answer, and it
is what 012 already sends.
