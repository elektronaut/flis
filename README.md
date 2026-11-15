# snags

`snags` is a local CLI where coding agents record anything that slowed down
development, look up what is already known about a failure, and close entries
once the root cause is fixed.

An **entry** is one problem: a symptom, how to get past it, and what is thought
to cause it. An **occurrence** is one time someone hit it. Recurrence is the
priority signal, so every encounter is recorded.

## Commands

```
snags search <text>... [--limit N] [--status S]
snags add <symptom>... [--workaround T] [--hypothesis T] [--fix T]
         [--subject S] [--silent] [--cost C] [--note T] [--project P] [--input FILE]
snags seen <id> [--note T] [--cost C]
snags show <id> [--history]
snags edit <id> [--symptom T] [--workaround T] [--hypothesis T] [--fix T]
          [--subject S] [--silent | --no-silent] [--ref R]
snags resolve <id> --where setup|project|upstream --ref R [--note T]
snags reopen <id>
snags list [--status S] [--project P] [--subject S] [--since D] [--sort rank|recent] [--limit N]
```

For the review:

```
snags accept <id> --note T
snags merge <id>... --into <id>
snags split <id> --occurrence <oid>... --symptom T
snags stats [--since D]
snags export [--format md|json] [--status S] [--max-tokens N]
snags import <file> [--dry-run] [--force] [--project-map FILE]
```

- `search` takes free text, such as a pasted error line. It looks for entries
  matching every word first, then any word, and says which.
- `add` always writes, then lists up to three similar entries. `snags add --help`
  describes each field. Long or quote-heavy text goes in a JSON file passed
  with `--input`.
- `seen` on a resolved entry reopens it and counts a regression.
- `list` shows open entries, silent ones first, then by score = weighted
  occurrences × distinct days × distinct projects, counting only occurrences
  inside `--since` (90 days by default). Open entries with a `ref` are listed
  separately as reported and waiting.
- `accept` marks a known problem with no fix planned; the reason is
  required. Accepted entries stay searchable and leave the ranking.
- `merge` moves the occurrences of duplicates to one entry and appends each
  source's text to its first occurrence's note. The source stays as a stub,
  hidden from `list`, `search` and `stats`, and its id leads to the target.
  There is no undo; `split` moves occurrences out into a new entry and
  repairs a wrong merge.
- `stats` counts entries by status, project and subject, and the open
  entries reported upstream and waiting.
- `export` writes complete entries with their occurrences in rank order and
  stops before the first entry that would go over `--max-tokens` (four
  characters per token), saying how many it left out. It masks well-known
  secret formats in its output: token prefixes (`ghp_`, `github_pat_`,
  `sk-`, `xox…`), AWS access key ids, `Authorization` and `Cookie` header
  values, credentials in URLs, private key blocks and `key=value`
  assignments whose key names a secret. The stored text is not changed.
- `import` reads a Markdown log with one entry per line,
  `YYYY-MM-DD · symptom · fix · project`, after a header ending in a `---`
  line. Each line becomes an open entry with one occurrence; indented lines
  under it become its note. Lines without exactly four fields are printed
  and skipped. A file imported before is refused unless `--force`.

Every command takes `--json`, which prints exactly one JSON object on stdout.
Nothing prompts. Exit codes: 0 success (empty results included), 1 internal or
I/O failure, 2 usage or validation error, 3 id not found.

## Review

`skills/snags-review/SKILL.md` is a skill for agents such as Claude Code that
reviews the log on request: it snapshots it with `stats` and `export`, merges
duplicates, ranks by recurrence, verifies the logged causes, and proposes a
disposition per problem. It fixes only what it is asked to fix.

## Storage

One SQLite database per machine, never synced:

1. `$SNAGS_DB`, if set
2. else `$XDG_DATA_HOME/snags/snags.sqlite3`
3. else `~/.local/share/snags/snags.sqlite3`

The directory is created with mode `0700` and the file with `0600`. The
database runs in WAL mode with a five second busy timeout, and each command is
one transaction, so agents running in parallel can write at once. In a
sandboxed session, the database directory must be listed in
`sandbox.filesystem.allowWrite`.

## License

MIT
