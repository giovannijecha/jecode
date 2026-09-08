Harden this ledger CLI and add filtering. Complete the implementation, regression
tests and README. Use one agent. Work only in this project; use the installed
Node.js standard library, with no dependencies, web access, external services,
delegated agents or changes to Git history. Do not inspect sibling directories.

Preserve `summarizeLedger(text, filters = {})` in `src/ledger.js`, and the result
shape `{ entries, totalCents, accounts: [{ account, entries, totalCents }] }`.

Requirements:

1. Parse the exact header `date,account,amount`, with an optional leading UTF-8
   BOM. Support LF and CRLF, quoted fields containing commas, doubled quotes and
   embedded newlines, and an optional final line ending. Reject malformed
   quoting, extra characters after a closing quote, blank records, wrong headers,
   and records with other than three fields. A header alone is a valid empty ledger.
2. Dates must be actual calendar dates in YYYY-MM-DD, years 0001 through 9999.
   Trim outer whitespace on each decoded field. Account must be nonempty;
   preserve its internal characters and case.
3. Amounts accept an optional minus sign, one or more ASCII digits, and optionally
   a decimal point followed by one or two digits. Reject plus signs, exponents,
   NaN, Infinity, missing digits, and extra decimal places. Convert exactly to
   integer cents. Reject unsafe integers for a row or any running account/overall
   sum. Normalize negative zero to zero. Do not use floating-point rounding.
4. Filters are optional `from`, `to` (inclusive dates) and `account` (exact
   case-sensitive match after trimming the filter). Reject invalid filters,
   unknown filter keys, empty account filters and inverted date ranges. Validate
   every input row before filtering, including excluded rows. Aggregate included
   rows only; return zero entries/total and an empty accounts array when none match.
   Sort accounts in ascending JavaScript string comparison order (not locale order).
5. CLI: `node src/cli.js FILE [--from DATE] [--to DATE] [--account NAME]`.
   Options may appear before or after FILE. Reject duplicates, missing option
   values, unknown flags, no file or extra positional arguments. `--help` alone
   prints usage and succeeds. Success emits exactly one JSON value plus a newline
   on stdout. Invalid arguments, unreadable files or invalid contents exit 1,
   emit a concise error on stderr with no stack trace, and leave stdout empty.
6. Add meaningful automated tests for parsing, exact money, filters and CLI
   failure paths. Update the README with the contract and examples. Run the full
   test suite, review your changes, and fix any issues you find before finishing.

Briefly report what changed and which checks actually passed. No need to ask
clarifying questions: the requirements above define the task.
