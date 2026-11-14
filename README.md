# flis

`flis` (Norwegian for splinter) is a local CLI where coding agents record
anything that slowed down development, look up what is already known about a
failure, and close entries once the root cause is fixed.

An **entry** is one problem: a symptom, how to get past it, and what is thought
to cause it. An **occurrence** is one time someone hit it. Recurrence is the
priority signal, so every encounter is recorded.

## Commands

```
flis search <text>... [--limit N] [--status S]
flis add <symptom>... [--workaround T] [--hypothesis T] [--fix T]
         [--subject S] [--silent] [--cost C] [--note T] [--project P] [--input FILE]
flis seen <id> [--note T] [--cost C]
flis show <id> [--history]
flis edit <id> [--symptom T] [--workaround T] [--hypothesis T] [--fix T]
          [--subject S] [--silent | --no-silent] [--ref R]
flis resolve <id> --where setup|project|upstream --ref R [--note T]
flis reopen <id>
flis list [--status S] [--project P] [--subject S] [--since D] [--sort rank|recent] [--limit N]
```

- `search` takes free text, such as a pasted error line. It looks for entries
  matching every word first, then any word, and says which.
- `add` always writes, then lists up to three similar entries. `flis add --help`
  describes each field. Long or quote-heavy text goes in a JSON file passed
  with `--input`.
- `seen` on a resolved entry reopens it and counts a regression.
- `list` shows open entries, silent ones first, then by score = weighted
  occurrences × distinct days × distinct projects, counting only occurrences
  inside `--since` (90 days by default). Open entries with a `ref` are listed
  separately as reported and waiting.

Every command takes `--json`, which prints exactly one JSON object on stdout.
Nothing prompts. Exit codes: 0 success (empty results included), 1 internal or
I/O failure, 2 usage or validation error, 3 id not found.

## Storage

One SQLite database per machine, never synced:

1. `$FLIS_DB`, if set
2. else `$XDG_DATA_HOME/flis/flis.sqlite3`
3. else `~/.local/share/flis/flis.sqlite3`

The directory is created with mode `0700` and the file with `0600`. The
database runs in WAL mode with a five second busy timeout, and each command is
one transaction, so agents running in parallel can write at once. In a
sandboxed session, the database directory must be listed in
`sandbox.filesystem.allowWrite`.

## License

MIT
