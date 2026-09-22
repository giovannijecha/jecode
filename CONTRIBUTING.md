# Contributing

Use Rust 1.95.0 and a native linker. Keep the dependency graph limited to Jecode:
the application, tests and build code use only the standard library and original
code. Do not copy or vendor third-party implementations.

Keep changes focused and modular. Separate rendering from effects. Preserve
explicit approvals, cancellation, cleanup, workspace boundaries and canonical
history. Changes to persistence must not replay tools or silently reinterpret
existing user data.

Use isolated fixtures for tests. Do not read real credentials or contact a model
to make an offline test pass. Add behavioral coverage for relevant failures and
run focused tests while working. At a meaningful change boundary run:

```text
cargo run --locked --offline --bin jecode-check -- check
cargo build --locked --offline --release --bin jecode
```

The check runner is owned verification code, included so a clean public checkout
can reproduce CI. Native platform APIs and any new unsupported boundaries belong
in the documentation. Code, comments, UI and documentation are written in English.

Explain the concrete problem, resulting behavior and validation in a pull request.
Report measurements with workload, model, effort, correctness and limitations.
Do not include credentials, user sessions, generated artifacts, local development
notes or agent instructions in a contribution.
