use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::clock;
use crate::commands::add::AddInput;
use crate::commands::edit::EditArgs;
use crate::commands::export::{ExportArgs, Format};
use crate::commands::list::{ListArgs, Sort};
use crate::commands::seen::SeenArgs;
use crate::commands::{self, Context};
use crate::db;
use crate::error::{Error, Result};
use crate::model::{Cost, FixLocation, Status};
use crate::output::Report;

#[derive(Debug, Parser)]
#[command(
    name = "flis",
    version,
    about = "Record what slowed down development, look up known problems, and close them once fixed",
    long_about = "Record what slowed down development, look up known problems, and close them once fixed.\n\n\
        An entry is one problem: a symptom, how to get past it, and what is thought to cause it. \
        An occurrence is one time someone hit it. When something fails unexpectedly, search first; \
        on a match use its workaround and run `flis seen <id>`, otherwise `flis add` once you are past it.",
    after_help = "Exit codes: 0 success (empty results too), 1 internal or I/O failure, \
        2 usage or validation error, 3 id not found.\n\
        Database: $FLIS_DB, else $XDG_DATA_HOME/flis/flis.sqlite3, else ~/.local/share/flis/flis.sqlite3.",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Print exactly one JSON object on stdout and nothing else
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Find known entries matching an error line or keywords, in every status
    #[command(
        long_about = "Find known entries matching an error line or keywords, in every status.\n\n\
        Paste the error text as it is: punctuation is ignored. Entries matching every word come first; \
        when none do, entries matching any of the words are shown, and the output says which. \
        Each hit shows its workaround, and a resolved hit what fixed it. No hits is not an error."
    )]
    Search(SearchArgs),
    /// Record a new problem and its first occurrence
    #[command(
        long_about = "Record a new problem and its first occurrence. Run `flis search` first; if it \
            finds the problem, run `flis seen <id>` instead.\n\n\
            Log anything that cost real time or is likely to recur: tooling failures, misleading \
            errors, environment quirks, dead ends (even with no workaround yet), your own repeated \
            mistakes, and problems in third-party tools. Do not log bugs in the project you are \
            working on (they go to its tracker), accomplishments or task status, secrets, \
            transcripts or large raw output.\n\n\
            `add` always writes, then lists up to three similar entries. A duplicate is fine: the \
            review merges them.",
        after_long_help = "Examples:\n  \
            flis add 'bundle install: \"Could not find gem rails-8.1 in locally installed gems\"' \\\n    \
            --subject bundler --workaround 'run bin/setup first; it sets BUNDLE_PATH' --cost major\n  \
            flis add --input entry.json\n\n\
            --input FILE reads one JSON object with any of: symptom, workaround, hypothesis, fix, \
            subject, silent (true/false), cost, note, project, agent. Use it for long or quote-heavy \
            text: write the file, then run a plain command. Arguments given on the command line win \
            over the file."
    )]
    Add(AddArgs),
    /// Record another occurrence of a known entry and print its workaround
    #[command(
        long_about = "Record another occurrence of a known entry and print its workaround.\n\n\
        On a resolved entry this is a regression: the entry is open again and its regression count \
        goes up. On an id that was merged away, the occurrence goes to the entry it was merged into."
    )]
    Seen(SeenCommand),
    /// Show an entry in full, with all its occurrences
    Show(ShowArgs),
    /// Change fields of an open entry; earlier values are kept
    #[command(
        long_about = "Change fields of an open entry, for example to record a better workaround \
        or the ref of an upstream report. The previous value of each changed field is kept and shown \
        by `flis show <id> --history`. An empty value clears an optional field."
    )]
    Edit(EditCommand),
    /// Mark an entry fixed, once the root fix has landed and the original failure was re-run
    #[command(
        long_about = "Mark an entry fixed. Run it only after the root cause is fixed and the \
        original failure was re-run and passed. A workaround never resolves an entry; record it with \
        `flis edit <id> --workaround` instead. An upstream report that is filed but not fixed stays \
        open: set its ref with `flis edit <id> --ref`."
    )]
    Resolve(ResolveArgs),
    /// Make a resolved or accepted entry open again
    Reopen(IdArg),
    /// Mark an open entry as a known problem with no fix planned
    #[command(
        long_about = "Mark an open entry as a known problem with no fix planned. --note gives the \
        reason and is required. Accepted entries are kept for lookup and still collect occurrences, \
        but leave the review ranking. Meant for the review; `flis reopen <id>` undoes it."
    )]
    Accept(AcceptArgs),
    /// Merge duplicate entries into one, moving their occurrences
    #[command(
        long_about = "Merge duplicate entries into one. The occurrences of each source entry move \
        to the target, and the source's own fields are appended to the note of its first \
        occurrence, so nothing is lost. The source stays as a stub that `list`, `search` and \
        `stats` leave out; its id still leads to the target. There is no undo: repair a wrong \
        merge with `flis split`."
    )]
    Merge(MergeArgs),
    /// Move occurrences that have a different cause into a new entry
    #[command(
        long_about = "Move occurrences that turn out to have a different cause into a new entry \
        with the given symptom and the same subject. Occurrence ids are shown by `flis show <id>` \
        (o117 or 117). The entry keeps at least one occurrence. Also repairs a wrong merge."
    )]
    Split(SplitArgs),
    /// Count entries by status, project and subject
    #[command(
        long_about = "Count entries by status, project and subject, and the open entries reported \
        upstream and waiting. Facts only, nothing inferred. Without --since every entry and \
        occurrence counts; with it, only entries seen in the window and their occurrences there. \
        Merged entries are left out."
    )]
    Stats(StatsArgs),
    /// Write complete entries in rank order, within a token budget, with secrets masked
    #[command(
        long_about = "Write complete entries with all their occurrences, in rank order: silent \
        first, then by score over the last 90 days, entries not seen in that window after them, \
        and open entries with a ref (reported, waiting) last. With --max-tokens it stops before the \
        first entry that would go over the budget, estimated at four characters per token, and \
        says how many were left out. Well-known secret formats are masked in the output (token \
        prefixes such as ghp_ and sk-, AWS key ids, Authorization and Cookie values, credentials \
        in URLs, private keys, secret=value assignments); the stored text is not changed."
    )]
    Export(ExportCommand),
    /// List entries, ranked by how much they cost recently
    #[command(
        long_about = "List entries, by default open ones, ranked: silent entries first, then by \
        score = weight × days × projects, then the most recently seen. Weight counts each occurrence \
        as 1 (minor), 3 (major) or 5 (blocker); days and projects are distinct counts. Only \
        occurrences inside --since count. Open entries with a ref (reported upstream, waiting on a \
        fix) are listed separately below the rest."
    )]
    List(ListCommand),
}

fn parse_id(raw: &str) -> std::result::Result<i64, String> {
    raw.trim()
        .trim_start_matches('#')
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| format!("{raw:?} is not an entry id (a positive number such as 12)"))
}

#[derive(Debug, Args)]
pub struct IdArg {
    /// Entry id
    #[arg(value_parser = parse_id)]
    pub id: i64,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum StatusFilter {
    Open,
    Resolved,
    Accepted,
    All,
}

impl StatusFilter {
    fn status(self) -> Option<Status> {
        match self {
            Self::Open => Some(Status::Open),
            Self::Resolved => Some(Status::Resolved),
            Self::Accepted => Some(Status::Accepted),
            Self::All => None,
        }
    }
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Free text, such as a pasted error line
    #[arg(required = true, value_name = "TEXT")]
    pub text: Vec<String>,
    /// Most hits to show
    #[arg(long, default_value_t = 10, value_name = "N")]
    pub limit: usize,
    /// Only entries in this status
    #[arg(long, value_enum, default_value_t = StatusFilter::All, value_name = "S")]
    pub status: StatusFilter,
}

#[derive(Debug, Args)]
pub struct AddArgs {
    /// What happened, including the literal error text.
    ///
    /// Describe the observation, not its cause: the command or action, and the exact error message
    /// or wrong result as printed. Quote it as one argument, or give several words. Keep it to what
    /// is needed to recognise the problem next time; no large output.
    #[arg(value_name = "SYMPTOM")]
    pub symptom: Vec<String>,

    /// How to get past it.
    ///
    /// The concrete command, flag or step that got you unblocked, written so the next agent can
    /// apply it directly. Leave it out if there is none yet.
    #[arg(long, value_name = "TEXT")]
    pub workaround: Option<String>,

    /// What you think causes it. Unverified.
    ///
    /// A guess is fine and expected; it is stored apart from the observation and checked before
    /// anyone acts on it.
    #[arg(long, value_name = "TEXT")]
    pub hypothesis: Option<String>,

    /// The root fix you would suggest.
    ///
    /// What would stop this happening at all, such as a settings change, a dotfile or hook, an
    /// instruction, or an upstream patch, as opposed to the workaround.
    #[arg(long, value_name = "TEXT")]
    pub fix: Option<String>,

    /// The tool or area, as one lowercase word.
    ///
    /// For example bundler, gh, git, sandbox, docker, mise, zsh. Reuse an existing subject where one
    /// fits; when the value is new, `add` lists the closest existing ones.
    #[arg(long, value_name = "WORD")]
    pub subject: Option<String>,

    /// The failure gave a plausible wrong answer with no error.
    ///
    /// Set it when nothing failed visibly: a command succeeded but did the wrong thing, or output
    /// looked right and was not. Silent entries rank first.
    #[arg(long)]
    pub silent: bool,

    /// How much time it cost this time. Defaults to minor.
    #[arg(long, value_enum, value_name = "C")]
    pub cost: Option<Cost>,

    /// How it showed up this time.
    ///
    /// Context for this occurrence: what you were doing, a variant of the error, a variant
    /// workaround. Stored on the occurrence, not the entry.
    #[arg(long, value_name = "TEXT")]
    pub note: Option<String>,

    /// The project it happened in. Defaults to the main repository's directory name.
    ///
    /// Every worktree of a repository reports the main repository's name. Outside a repository
    /// there is no project.
    #[arg(long, value_name = "P")]
    pub project: Option<String>,

    /// Who hit it. Defaults to $FLIS_AGENT, else claude-code.
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,

    /// Read the fields from a JSON file instead (see below).
    #[arg(long, value_name = "FILE")]
    pub input: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct SeenCommand {
    /// Entry id
    #[arg(value_parser = parse_id)]
    pub id: i64,
    /// How it showed up this time: the error text, the context, a variant workaround
    #[arg(long, value_name = "TEXT")]
    pub note: Option<String>,
    /// How much time it cost this time. Defaults to minor
    #[arg(long, value_enum, value_name = "C")]
    pub cost: Option<Cost>,
    /// The project it happened in. Defaults to the main repository's directory name
    #[arg(long, value_name = "P")]
    pub project: Option<String>,
    /// Who hit it. Defaults to $FLIS_AGENT, else claude-code
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Entry id
    #[arg(value_parser = parse_id)]
    pub id: i64,
    /// Also show earlier values of changed fields
    #[arg(long)]
    pub history: bool,
}

#[derive(Debug, Args)]
pub struct EditCommand {
    /// Entry id
    #[arg(value_parser = parse_id)]
    pub id: i64,
    /// What happened, including the literal error text
    #[arg(long, value_name = "TEXT")]
    pub symptom: Option<String>,
    /// How to get past it
    #[arg(long, value_name = "TEXT")]
    pub workaround: Option<String>,
    /// Suspected cause, unverified
    #[arg(long, value_name = "TEXT")]
    pub hypothesis: Option<String>,
    /// Suggested root fix
    #[arg(long, value_name = "TEXT")]
    pub fix: Option<String>,
    /// The tool or area, as one lowercase word
    #[arg(long, value_name = "WORD")]
    pub subject: Option<String>,
    /// The failure gave a plausible wrong answer with no error
    #[arg(long, overrides_with = "no_silent")]
    pub silent: bool,
    /// The failure did show an error
    #[arg(long, overrides_with = "silent")]
    pub no_silent: bool,
    /// Link to the issue, PR, commit, file or rule that tracks it, such as a filed upstream report
    #[arg(long = "ref", value_name = "R")]
    pub reference: Option<String>,
}

#[derive(Debug, Args)]
pub struct ResolveArgs {
    /// Entry id
    #[arg(value_parser = parse_id)]
    pub id: i64,
    /// Where the fix landed
    #[arg(long = "where", value_enum, value_name = "WHERE")]
    pub location: FixLocation,
    /// The commit, PR, issue, file or rule with the fix
    #[arg(long = "ref", value_name = "R")]
    pub reference: String,
    /// What was done, and how the original failure was re-run
    #[arg(long, value_name = "TEXT")]
    pub note: Option<String>,
}

#[derive(Debug, Args)]
pub struct AcceptArgs {
    /// Entry id
    #[arg(value_parser = parse_id)]
    pub id: i64,
    /// Why it is accepted: why no fix is planned
    #[arg(long, value_name = "TEXT")]
    pub note: String,
}

#[derive(Debug, Args)]
pub struct MergeArgs {
    /// The entries to merge away
    #[arg(required = true, value_parser = parse_id, value_name = "ID")]
    pub ids: Vec<i64>,
    /// The entry that keeps them
    #[arg(long, value_parser = parse_id, value_name = "ID")]
    pub into: i64,
}

fn parse_occurrence_id(raw: &str) -> std::result::Result<i64, String> {
    raw.trim()
        .trim_start_matches(['o', 'O'])
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| format!("{raw:?} is not an occurrence id (such as o117)"))
}

#[derive(Debug, Args)]
pub struct SplitArgs {
    /// Entry id
    #[arg(value_parser = parse_id)]
    pub id: i64,
    /// An occurrence to move, such as o117; repeat it or give several
    #[arg(long, required = true, num_args = 1.., value_parser = parse_occurrence_id, value_name = "OID")]
    pub occurrence: Vec<i64>,
    /// What happened in the moved occurrences
    #[arg(long, value_name = "TEXT")]
    pub symptom: String,
}

#[derive(Debug, Args)]
pub struct StatsArgs {
    /// Only count occurrences since: 90d, 2w, 36h, a date (2026-08-01) or a timestamp
    #[arg(long, value_name = "D")]
    pub since: Option<String>,
}

#[derive(Debug, Args)]
pub struct ExportCommand {
    /// Output format; json prints the same object as --json
    #[arg(long, value_enum, default_value_t = Format::Md)]
    pub format: Format,
    /// Only entries in this status
    #[arg(long, value_enum, default_value_t = StatusFilter::Open, value_name = "S")]
    pub status: StatusFilter,
    /// Stop before the output would go over this many tokens (four characters each)
    #[arg(long, value_name = "N")]
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Args)]
pub struct ListCommand {
    /// Only entries in this status
    #[arg(long, value_enum, default_value_t = StatusFilter::Open, value_name = "S")]
    pub status: StatusFilter,
    /// Only entries seen in this project
    #[arg(long, value_name = "P")]
    pub project: Option<String>,
    /// Only entries with this subject
    #[arg(long, value_name = "WORD")]
    pub subject: Option<String>,
    /// The window that counts: 90d, 2w, 36h, a date (2026-08-01) or a timestamp
    #[arg(long, default_value = "90d", value_name = "D")]
    pub since: String,
    /// Order of the entries
    #[arg(long, value_enum, default_value_t = Sort::Rank)]
    pub sort: Sort,
    /// Most entries to show in each group
    #[arg(long, default_value_t = 25, value_name = "N")]
    pub limit: usize,
}

/// Runs flis with the given arguments and returns the exit code.
pub fn run(args: Vec<OsString>) -> i32 {
    let wants_json = args.iter().skip(1).any(|arg| arg == "--json");
    let cli = match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error) => {
            use clap::error::ErrorKind::{
                DisplayHelp, DisplayHelpOnMissingArgumentOrSubcommand, DisplayVersion,
            };
            if matches!(error.kind(), DisplayHelp | DisplayVersion) || !wants_json {
                let _ = error.print();
                return if matches!(error.kind(), DisplayHelp | DisplayVersion) {
                    0
                } else if error.kind() == DisplayHelpOnMissingArgumentOrSubcommand {
                    2
                } else {
                    error.exit_code()
                };
            }
            let message = error.render().to_string();
            let message = message.trim().trim_start_matches("error: ");
            return fail(&Error::usage(message), true);
        }
    };
    let json = cli.json
        || matches!(
            &cli.command,
            Command::Export(ExportCommand {
                format: Format::Json,
                ..
            })
        );
    match execute(cli, json) {
        Ok(output) => {
            let mut stdout = std::io::stdout().lock();
            let text = if json { output.json } else { output.text };
            let _ = stdout.write_all(text.as_bytes());
            0
        }
        Err(error) => fail(&error, json),
    }
}

fn fail(error: &Error, json: bool) -> i32 {
    if json {
        let body = serde_json::json!({
            "error": { "kind": error.kind(), "code": error.exit_code(), "message": error.to_string() }
        });
        println!("{body}");
    } else {
        eprintln!("flis: {error}");
    }
    error.exit_code()
}

struct Output {
    json: String,
    text: String,
}

fn output(report: &impl Report) -> Result<Output> {
    let json = serde_json::to_string(report).map_err(|error| Error::Internal(error.to_string()))?;
    Ok(Output {
        json: json + "\n",
        text: report.text(),
    })
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn execute(cli: Cli, json: bool) -> Result<Output> {
    let ctx = Context {
        now: clock::now_from_env(env("FLIS_NOW").as_deref())?,
        cwd: crate::context::current_dir(),
        agent: env("FLIS_AGENT"),
        session: env("CLAUDE_CODE_SESSION_ID"),
    };
    let path = db::resolve_path(env)?;
    let mut conn = db::open(&path)?;
    dispatch(&mut conn, &ctx, cli.command, json).map_err(|error| db::storage_error(&path, error))
}

fn dispatch(
    conn: &mut rusqlite::Connection,
    ctx: &Context,
    command: Command,
    json: bool,
) -> Result<Output> {
    match command {
        Command::Search(args) => {
            let text = args.text.join(" ");
            output(&commands::search::run(
                conn,
                &text,
                args.status.status(),
                args.limit,
            )?)
        }
        Command::Add(args) => {
            let base = match &args.input {
                Some(path) => AddInput::from_file(path)?,
                None => AddInput::default(),
            };
            let symptom = (!args.symptom.is_empty()).then(|| args.symptom.join(" "));
            let given = AddInput {
                symptom,
                workaround: args.workaround,
                hypothesis: args.hypothesis,
                fix: args.fix,
                subject: args.subject,
                silent: args.silent.then_some(true),
                cost: args.cost,
                note: args.note,
                project: args.project,
                agent: args.agent,
            };
            output(&commands::add::run(conn, ctx, given.over(base))?)
        }
        Command::Seen(args) => {
            let seen = SeenArgs {
                id: args.id,
                note: args.note.as_deref(),
                cost: args.cost,
                project: args.project.as_deref(),
                agent: args.agent.as_deref(),
            };
            output(&commands::seen::run(conn, ctx, &seen)?)
        }
        Command::Show(args) => output(&commands::show::run(conn, args.id, args.history)?),
        Command::Edit(args) => {
            let silent = match (args.silent, args.no_silent) {
                (true, _) => Some(true),
                (_, true) => Some(false),
                _ => None,
            };
            let edit = EditArgs {
                id: args.id,
                symptom: args.symptom,
                workaround: args.workaround,
                hypothesis: args.hypothesis,
                fix: args.fix,
                subject: args.subject,
                silent,
                reference: args.reference,
            };
            output(&commands::edit::run(conn, ctx, edit)?)
        }
        Command::Resolve(args) => output(&commands::resolve::resolve(
            conn,
            ctx,
            args.id,
            args.location,
            &args.reference,
            args.note.as_deref(),
        )?),
        Command::Reopen(args) => output(&commands::resolve::reopen(conn, ctx, args.id)?),
        Command::Accept(args) => {
            output(&commands::resolve::accept(conn, ctx, args.id, &args.note)?)
        }
        Command::Merge(args) => output(&commands::merge::run(conn, ctx, &args.ids, args.into)?),
        Command::Split(args) => output(&commands::split::run(
            conn,
            ctx,
            args.id,
            &args.occurrence,
            &args.symptom,
        )?),
        Command::Stats(args) => output(&commands::stats::run(conn, ctx, args.since.as_deref())?),
        Command::Export(args) => {
            let export = ExportArgs {
                format: if json { Format::Json } else { args.format },
                status: args.status.status(),
                max_tokens: args.max_tokens,
            };
            output(&commands::export::run(conn, ctx, &export)?)
        }
        Command::List(args) => {
            let list = ListArgs {
                status: args.status.status(),
                project: args.project.as_deref(),
                subject: args.subject.as_deref(),
                since: &args.since,
                sort: args.sort,
                limit: args.limit,
            };
            output(&commands::list::run(conn, ctx, &list)?)
        }
    }
}
