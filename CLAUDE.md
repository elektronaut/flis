# snags

A local CLI where coding agents record what slowed down development, look up known problems, and close them once fixed. One package: a library and a thin binary.

- `src/cli.rs`: clap definitions, `--json` handling and exit codes. `snags add --help` is the field guidance agents read, so keep it complete.
- `src/commands/`: one module per command (`add`, `seen`, `show`, `edit`, `resolve` with `reopen` and `accept`, `list`, `search`, `merge`, `split`, `stats`, `export`, `import`). Each returns a report that renders as text or JSON.
- `src/db.rs`: database path, permissions, WAL, busy timeout and schema migrations through `PRAGMA user_version`.
- `src/store.rs`: queries on entries, occurrences and `entry_history`, and the FTS rebuild.
- `src/search.rs`: tokenising free text into quoted FTS5 terms and the all-words, then any-word search.
- `src/rank.rs`: the ranking factors and order.
- `src/mask.rs`: masking of secret formats in `export` output. Stored text is never masked.
- `src/import.rs`: the parser for the old Markdown log and its project normalisation. Keep it generic: no mappings between particular names.
- `src/context.rs`: project, branch and cwd from git. `src/clock.rs`: UTC timestamps, `SNAGS_NOW`, `--since`.
- `tests/cli/`: black-box tests that run the binary; `tests/fixtures/` holds their input files.
- `skills/snags-review/SKILL.md`: the review skill. The repository is public, so keep it generic: no personal names, paths or setups.

## Conventions

- Conventional Commits, scoped by area: `feat(cli): …`, `fix(db): …`, `test(search): …`. Before 1.0.0 a `feat:` bumps the minor version (0.x.0); fixes bump the patch.
- `cargo test` and `cargo clippy --all-targets` must be clean before committing; run `cargo fmt --all`. CI runs the tests with `cargo nextest run --profile ci`.
- Every write and its FTS rebuild happen in one transaction; writers start it with `BEGIN IMMEDIATE`.
- A schema change is a new element at the end of `MIGRATIONS` in `src/db.rs`; never edit an existing one.
- Tests never touch the real database or home directory: always set `SNAGS_DB` to a temp path, and `HOME` too (the `tests/cli/common.rs` helpers do both). Use `SNAGS_NOW` to pin the clock.
- Never use real project, client or app names in code, tests, docs or commit messages, not even ones seen in local config or logs. Use made-up names like `acme`, `nova`, `shop` or `mono`.
