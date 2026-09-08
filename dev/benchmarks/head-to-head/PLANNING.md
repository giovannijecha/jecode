# Planning experiment

This is a screening experiment, not a claim that either agent is faster in
general. Freeze the tasks, evaluators, sources, harness and sequence before
starting any live run. Preserve every attempt, including incomplete outputs.

The baseline uses the current production system prompt. The grouped variant
adds one instruction to collect related, non-overlapping edits, inspect results
before dependent changes, and retain necessary tests and failure checks.
`scenarios.py ROOT` creates a separately hashed source copy; it never edits the
checkout or baseline. Only `src/prompt.ts` differs between variants.
Codex retains its native instructions and tool interface.

`PLANNING-EXPERIMENT.json` declares six serial trials: baseline/Codex/grouped on
cache, then grouped/Codex/baseline on planner. Each task receives one trial per
client/configuration. Reversing the Jecode order across tasks reduces a simple
order confound but is not repeated or randomized statistical evidence. Native
provider prompt-cache state and backend load remain uncontrolled.

The cache task tests debugging of asynchronous generations, expiry and per-waiter
cancellation. The planner task exercises integration across ten implementation
modules, graph validation and CLI behavior. It is a larger fixture than cache,
not evidence for performance on a genuinely large production repository.
Each has a separate evaluator outside the agent workspace. Verify that the
original fixture fails relevant checks before the trials, without modifying the
scorer in response to model output.

Primary criteria are completed task plus all external checks passing. Review
project tests, README, changes outside scope and saved conversation integrity.
Then compare submission-to-completion time, model requests, output tokens,
preparation time, transport failures and adjacent edit-only responses. Fewer
tests or incomplete work are not speed gains. Raw tool counts are not comparable
between the two native tool interfaces.

```sh
python3 scenarios.py ROOT
python3 batch.py --root ROOT --prefix planning --plan PLANNING-EXPERIMENT.json
python3 verify.py ROOT --own-tests
python3 analyze.py ROOT
```

Retain the production prompt if results are mixed. Any proposed promotion needs
additional fresh tasks and repeated trials; do not tune on these outputs and
then report the same tasks as independent validation. Interruption/resume is a
separate recovery experiment and must not be pooled with uninterrupted timings.
