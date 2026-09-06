# Token vocabulary

`assets/tokenizers/o200k-base.tiktoken.gz` contains the unmodified OpenAI `o200k_base` merge ranks,
compressed with gzip. It is data, not a runtime library. The byte-pair counter,
bounded loading, caching, and provider budgeting are owned TypeScript code.

- [Source vocabulary](https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken)
- [Encoding definition](https://github.com/openai/tiktoken/blob/0.12.0/tiktoken_ext/openai_public.py)
- [Model mappings](https://github.com/openai/tiktoken/blob/0.12.0/tiktoken/model.py)
- [MIT attribution](../../assets/tokenizers/LICENSE), shipped beside the vocabulary
- Uncompressed SHA-256: `446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d`

The explicit development command `node scripts/import-tokenizer.ts` downloads
only that pinned vocabulary, checks its checksum, and regenerates the compressed
asset. Normal build, install, and execution never download tokenizer data.
The runtime validates the checksum before parsing and loads the ranks lazily.
The gzip asset is about 1.69 MB; the package guard permits at most 2.5 MB total.

`dev/test-support/tokenizer-fixtures.json` contains synthetic ordinary-text
counts generated independently with Python `tiktoken==0.12.0`, including Unicode,
code, whitespace, contractions, and special-looking literal text. Python and
tiktoken are not required to run the fixture tests or use Jecode.

OpenAI Account aliases may not have a published model-to-encoding mapping.
Using this vocabulary for those aliases is a reference estimate, not proof of
their internal tokenizer. See [context measurement](../../docs/CONTEXT.md) for
protocol allowances, calibration, fallback behavior, and validation limits.
