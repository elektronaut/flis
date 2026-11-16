---
name: snags-review
description: Review the snags log of what slowed down development. Snapshots it, merges duplicates, ranks by recurrence, verifies causes, and proposes a disposition per problem (fix in the shared setup or a project, report upstream, accept, or needs a reproduction). Use when the user asks to review papercuts, snags or what keeps slowing agents down, or for the classification pass after `snags import`.
---

# snags-review

`snags` is a local log where coding agents record what slowed them down. An
**entry** is one problem (symptom, workaround, hypothesis, suggested fix); an
**occurrence** is one time someone hit it, shown with an `o` prefix (`o117`).
Recurrence is the priority signal. This skill turns the log into a short list
of decisions.

## What a review may change

A review request is not a fix mandate. The review proposes; it fixes, edits
or changes the status of an entry only where the user asked for it. When in
doubt, put it in the report and ask.

- Merging duplicates and splitting occurrences is part of the review. Say
  what you merged and why.
- `snags accept` and `snags resolve` change what later reviews see. Run them
  only for entries the user agreed to.
- Any session may `snags resolve`, but only after the root fix has landed and
  the original failure was re-run and passed. A workaround never resolves an
  entry; a better workaround goes in with `snags edit <id> --workaround`.

## 1. Snapshot

```sh
snags stats
snags export --max-tokens 20000
```

`stats` gives counts by status, project and subject, and the number of open
entries that are reported upstream and waiting. A long tail of subjects
used once usually means the same tool under different names: note it, and
fix it with `snags edit <id> --subject`. `export` writes complete entries
with their occurrences in rank order, masks well-known secret formats, and
says how many entries it left out. Raise the budget or narrow it with
`--status` rather than reviewing half a picture without saying so.

Cite entry ids (`#12`) and occurrence ids (`o117`) for every claim, so each
one can be checked.

## 2. Group by mechanism, merge, split

Group entries by **failure mechanism**: the thing that actually goes wrong,
such as "the command guard rejects any compound command". Not by project and
not by subject; the same mechanism shows up in many projects and under
several subjects, and one subject covers unrelated mechanisms.

- Duplicates of one mechanism: `snags merge <id>... --into <id>`. Keep the
  entry with the best symptom and workaround as the target. The sources'
  text is kept on their first occurrence, and their ids still lead to the
  target. There is no undo.
- An occurrence that turns out to have a different cause:
  `snags split <id> --occurrence <oid>... --symptom "…"`. This also repairs a
  wrong merge.

## 3. Rank

Silent failures first: they gave a plausible wrong answer with no error,
so nobody notices them. Then by score, as `snags list` and `snags export`
order them: weighted occurrences × distinct days × distinct projects. For
each entry or group, show the occurrence count, distinct days and distinct
projects next to the disposition.

Accepted entries are left out of the ranking. Occurrences still count, so
an accepted entry that keeps coming back can be reconsidered.

### Single occurrences

Most entries are seen once, and they all score the same, so the score
cannot order them. Rank them separately, below the entries that recurred:

1. Cost of the one incident: silent, data loss, production impact or hours
   lost first. Agents log nearly everything as minor, so judge it from the
   text, and reclassify a cost that is clearly wrong with
   `snags edit <id> --cost <C> [--occurrence <oid>...]`.
2. Cost of the fix: a fix of a line or two in one of the user's own
   repositories, verified as still needed, is cheaper to make than to keep
   reviewing. Propose it as a fix now.
3. Staleness: an entry seen once, more than 30 days ago, not silent and with
   no `ref`, is proposed for `snags accept` as stale. Recurrences mostly come
   within days, and a new occurrence brings an accepted entry back. Silent
   entries are exempt: nobody notices them recur.

Report the singles in these buckets: fix now, high cost, waiting or needing
a reproduction, stale, and not yet 30 days old.

## 4. Verify before acting

A logged cause is a hypothesis, written by an agent in the middle of
another task. Before acting on an entry:

- Reproduce the failure, or read the thing it blames: the hook, the
  setting, the config file, the tool's source or changelog.
- Search for older entries with a different diagnosis of the same symptom
  (`snags search "<error text>"`, `snags list --status all`). Two agents often
  blamed different things.
- The `session` on an occurrence identifies the session whose transcript
  shows what actually happened. Read it when the entry is thin or the cause
  is disputed.

Keep track of what you verified and what is still a hypothesis; the report
needs both.

## 5. Propose one disposition per entry or group

| Disposition | When | What happens |
| --- | --- | --- |
| Fix in the shared setup | Dotfiles, agent instructions, settings, hooks or skills cause it or can prevent it | Fix, re-run, `snags resolve <id> --where setup --ref <commit>` |
| Fix in a project | One of the user's repositories causes it | Fix there, re-run, `snags resolve <id> --where project --ref <commit or PR>` |
| Report upstream | Someone else's software, the agent tool included | Draft the issue for the user to file. The entry stays open; once filed, `snags edit <id> --ref <issue url>` |
| Accept | A known problem with no fix planned | `snags accept <id> --note "<why>"` |
| Needs a reproduction | It cannot be verified from what is logged | Say what would reproduce it; leave it open |

## 6. Fix, when asked

- Prefer changing the environment over adding an instruction. A setting,
  hook, wrapper, script or default that makes the failure impossible beats a
  line asking agents to remember. Add an instruction only when nothing in
  the environment can change.
- Work in small batches. After each fix, re-run the original failure from
  the entry and its occurrences, and confirm it no longer happens.
- When loosening a guard or check, also prove it still stops what it exists
  to stop: run one case it must still refuse and show that it does.

## 7. Resolve

Resolve only entries whose fix was verified by re-running the failure, each
with `--where` and a `--ref` to the commit, PR, file or rule with the fix:

```sh
snags resolve <id> --where setup --ref <commit> --note "<what changed; how it was re-run>"
```

## Reported, waiting

Open entries with a `ref` are reported upstream and waiting on a fix;
`snags list` shows them as a separate group. Check each reference: if the fix
has shipped, upgrade to it, re-run the original failure, and resolve the ones that
no longer reproduce (`--where upstream --ref <release or PR>`). Leave the
rest open and say what they are waiting on.

## First review after an import

Entries imported with `snags import` all arrive open, many of them already
fixed or long accepted. The first review is a classification pass: merge
the obvious duplicates, then propose which entries look already fixed and
which look like accepted gotchas, with the evidence for each. Change no
status until the user has gone through that list.

## Report

End with:

1. A table of the entries or groups, ranked: ids, symptom in a few words,
   count, distinct days, distinct projects, disposition.
2. What was fixed, and how each fix was verified (the command re-run and
   what it printed).
3. What stayed open, and why.
4. What was verified and what is still a hypothesis.
5. Merges and splits made, and anything else changed in snags.
