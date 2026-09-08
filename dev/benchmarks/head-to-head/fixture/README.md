# Ledger pilot

A small Node.js ledger summarizer. No dependencies or build step.

Run `npm test` and `node src/cli.js ledger.csv`.
The current parser supports only simple unquoted CSV and does not validate input.

Keep `summarizeLedger(text, filters = {})` exported from `src/ledger.js`.
