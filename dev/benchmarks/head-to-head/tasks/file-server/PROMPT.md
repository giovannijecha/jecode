Finish the local static file server in this repository. Keep the existing public
exports and tests, use only Node standard-library modules, and stay within this
request. Do not add a UI, dependencies, uploads, authentication or deployment.
Implement the behavior, add meaningful regression tests, update the README, and
report verified outcomes and any limitations. Do not commit or publish.

`await createHandler(root)` must return a Node HTTP request listener after
validating that root is an existing directory (resolve its real path once).
The listener serves only existing regular files under that directory:

- Support GET and HEAD. Other methods return 405 with `Allow: GET, HEAD`.
  HEAD returns the same headers as a full GET but never sends a body; Range is
  ignored for HEAD. Missing files and directories return 404; there is no
  implicit index file or directory listing.
- Ignore the query string. Decode the raw path once with decodeURIComponent;
  malformed escapes and NUL return 400. Reject decoded `..` path segments,
  backslashes, and symlinks in any component below root with 403. Enforce the
  root boundary. Escaped spaces and Unicode filenames work.
- Successful full responses have Content-Length, Last-Modified (HTTP date),
  and a weak ETag derived consistently from file size and mtime. Use these MIME
  types: .html text/html, .css text/css, .js text/javascript, .json application/json,
  .svg image/svg+xml, .txt text/plain; add `; charset=utf-8` for those types.
  Everything else uses application/octet-stream. Include Accept-Ranges: bytes.
- If-None-Match supports `*` and a comma-separated list, with weak comparison;
  a match returns 304 with no body. When If-None-Match is present it takes
  precedence over If-Modified-Since, including when the tag does not match.
  Otherwise a valid If-Modified-Since at or after the file modification time
  rounded down to seconds returns 304. Invalid dates are ignored.
- GET supports a single byte range: `bytes=start-end`, `bytes=start-`, or
  `bytes=-suffixLength`. Clip the end to the file size. A valid satisfiable
  range returns 206, correct Content-Range and Content-Length, and exactly the
  selected bytes. Unsatisfiable, malformed, reversed, zero suffix, multiple,
  or numerically unsafe byte ranges return 416 with `Content-Range: bytes */SIZE`.
  Unknown range units are ignored (full 200). All ranges on an empty file are
  unsatisfiable. Evaluate conditional 304 behavior before Range.
- If-Range permits a range only for a valid HTTP date at or after Last-Modified.
  The server only provides weak ETags, so entity-tag If-Range values (including
  the server's weak ETag) and invalid dates cause Range to be ignored.
- Stream file content with bounded memory. A client disconnect must release
  the file stream and must not crash the server or prevent later requests.
  Failures before sending headers produce a bounded 500 response; failures after
  headers close the response. Never expose a filesystem path or stack trace in
  an error response. HEAD error responses also have no body.

Keep `startServer(root, {port=0, host='127.0.0.1'}={})` returning
`{server, port, close}` where close is awaitable and safe to call twice.
`node src/cli.js ROOT [--port PORT]` starts on loopback, prints exactly one JSON
line with the selected port, and shuts down cleanly on SIGINT/SIGTERM. Reject
unknown flags, missing values and ports outside 0..65535 with nonzero exit.
No external services are needed to implement or verify this task.
