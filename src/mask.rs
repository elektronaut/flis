//! Masks well-known secret formats in text that leaves the database.

use std::sync::LazyLock;

use regex::{Captures, Regex};

pub const MASK: &str = "[masked]";

struct Rule {
    regex: Regex,
    /// The replacement for a match, or `None` to leave it as it is.
    replace: fn(&Captures<'_>) -> Option<String>,
}

fn rule(pattern: &str, replace: fn(&Captures<'_>) -> Option<String>) -> Rule {
    Rule {
        regex: Regex::new(pattern).expect("mask pattern"),
        replace,
    }
}

fn group<'a>(caps: &'a Captures<'_>, index: usize) -> &'a str {
    caps.get(index).map_or("", |m| m.as_str())
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    vec![
        rule(
            r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY( BLOCK)?-----.*?(?:-----END [A-Z0-9 ]*PRIVATE KEY( BLOCK)?-----|\z)",
            |_| Some("[masked private key]".to_owned()),
        ),
        rule(
            r#"(?im)\b(proxy-authorization|authorization|set-cookie|cookie|x-api-key|api-key)(\s*:[ \t]*)(?:((?:bearer|basic|token|digest)[ \t]+))?([^\r\n"']*[^\r\n"'\s])"#,
            |caps| {
                let value = group(caps, 4);
                (!value.starts_with(MASK)).then(|| {
                    format!(
                        "{}{}{}{MASK}",
                        group(caps, 1),
                        group(caps, 2),
                        group(caps, 3)
                    )
                })
            },
        ),
        rule(
            r"\b([a-zA-Z][a-zA-Z0-9+.-]*://)([^\s/@:?#]*)(?::([^\s/@]*))?@",
            |caps| {
                let (scheme, user) = (group(caps, 1), group(caps, 2));
                match caps.get(3).map(|m| m.as_str()) {
                    Some(password) if !password.is_empty() && password != MASK => {
                        Some(format!("{scheme}{user}:{MASK}@"))
                    }
                    Some(_) => None,
                    None => looks_like_token(user).then(|| format!("{scheme}{MASK}@")),
                }
            },
        ),
        rule(
            r#"(?i)([a-z0-9_.-]*(?:secret|passw(?:or)?d|passphrase|api[_-]?key|private[_-]?key|access[_-]?key|auth[_-]?key|token)[a-z0-9_.-]*)([ \t]*=[ \t]*)("[^"\r\n]*"|'[^'\r\n]*'|[^\s"'&;,]+)"#,
            |caps| {
                let (key, separator, value) = (group(caps, 1), group(caps, 2), group(caps, 3));
                if !names_a_secret(key) {
                    return None;
                }
                let (quote, inner) = match value.chars().next() {
                    Some(q @ ('"' | '\'')) => (q.to_string(), &value[1..value.len() - 1]),
                    _ => (String::new(), value),
                };
                if is_placeholder(inner) {
                    return None;
                }
                Some(format!("{key}{separator}{quote}{MASK}{quote}"))
            },
        ),
        rule(
            r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|glpat-[A-Za-z0-9_-]{20,}|xox[abposr]-[A-Za-z0-9-]{10,}|xapp-[A-Za-z0-9-]{10,}|(?:AKIA|ASIA|ABIA|ACCA)[A-Z0-9]{16}|[rs]k_(?:live|test)_[A-Za-z0-9]{16,}|npm_[A-Za-z0-9]{36}|AIza[A-Za-z0-9_-]{35})\b",
            |_| Some(MASK.to_owned()),
        ),
        rule(r"\bsk-[A-Za-z0-9_-]{20,}", |caps| {
            let tail = &group(caps, 0)[3..];
            let digits = tail.chars().any(|c| c.is_ascii_digit());
            let letters = tail.chars().any(|c| c.is_ascii_alphabetic());
            (digits && letters).then(|| MASK.to_owned())
        }),
        rule(
            r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
            |_| Some(MASK.to_owned()),
        ),
    ]
});

/// A user part of a URL that is a token rather than a name: long, with
/// letters and digits.
fn looks_like_token(user: &str) -> bool {
    user.len() >= 16
        && user.chars().any(|c| c.is_ascii_digit())
        && user.chars().any(|c| c.is_ascii_alphabetic())
}

fn names_a_secret(key: &str) -> bool {
    let key = key.trim_start_matches(['-', '.']).to_ascii_lowercase();
    let not_the_value = [
        "file", "path", "dir", "env", "var", "cmd", "command", "name", "id", "length", "len",
        "type", "url", "count",
    ];
    if key
        .rsplit(['_', '-', '.'])
        .next()
        .is_some_and(|last| not_the_value.contains(&last))
    {
        return false;
    }
    let strong = [
        "secret",
        "password",
        "passwd",
        "passphrase",
        "apikey",
        "api_key",
        "api-key",
        "privatekey",
        "private_key",
        "private-key",
        "accesskey",
        "access_key",
        "access-key",
        "authkey",
        "auth_key",
        "auth-key",
    ];
    strong.iter().any(|word| key.contains(word)) || key.ends_with("token")
}

fn is_placeholder(value: &str) -> bool {
    value.is_empty()
        || value.starts_with(['$', '<', '%', '*', '{'])
        || value.starts_with(MASK)
        || ["true", "false", "null", "none", "nil"].contains(&value.to_ascii_lowercase().as_str())
}

/// The text with every recognised secret replaced, and the number of
/// replaced spans.
pub fn mask(text: &str) -> (String, usize) {
    let mut out = text.to_owned();
    let mut count = 0;
    for rule in RULES.iter() {
        if !rule.regex.is_match(&out) {
            continue;
        }
        out = rule
            .regex
            .replace_all(&out, |caps: &Captures<'_>| match (rule.replace)(caps) {
                Some(replacement) => {
                    count += 1;
                    replacement
                }
                None => group(caps, 0).to_owned(),
            })
            .into_owned();
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn masked(text: &str) -> String {
        mask(text).0
    }

    fn unchanged(text: &str) {
        assert_eq!(mask(text), (text.to_owned(), 0), "{text}");
    }

    #[test]
    fn masks_token_prefixes() {
        let cases = [
            (
                "token ghp_abcdefghijklmnopqrstuvwxyz0123456789 expired",
                "token [masked] expired",
            ),
            ("gho_ABCDEFGHIJKLMNOPQRSTUVWX12", "[masked]"),
            (
                "ghs_0123456789abcdefghijABCD in the remote",
                "[masked] in the remote",
            ),
            (
                "use github_pat_11ABCDEFG0123456789_abcdefghijKLMNOP now",
                "use [masked] now",
            ),
            ("glpat-abcdefghij0123456789", "[masked]"),
            (
                "key sk-ant-api03-AbCdEf0123456789xyzXYZ-_abc was rejected",
                "key [masked] was rejected",
            ),
            ("sk-proj-0123456789abcdefghijklmn", "[masked]"),
            (
                "slack xoxb-1234567890-0987654321-AbCdEfGhIj",
                "slack [masked]",
            ),
            ("xoxp-12345678901-abcdefghij", "[masked]"),
            (
                "AWS key AKIAIOSFODNN7EXAMPLE leaked",
                "AWS key [masked] leaked",
            ),
            ("ASIAABCDEFGHIJKLMNOP", "[masked]"),
            ("sk_live_0123456789abcdefABCD", "[masked]"),
            ("npm_abcdefghijklmnopqrstuvwxyz0123456789", "[masked]"),
            ("AIzaSyA-0123456789abcdefghijklmnopqrstu", "[masked]"),
            (
                "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
                "jwt [masked]",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(masked(input), expected, "{input}");
            assert_eq!(mask(input).1, 1, "{input}");
        }
    }

    #[test]
    fn masks_header_values() {
        assert_eq!(
            masked("curl -H \"Authorization: Bearer abc.def-123\" https://api.example.com"),
            "curl -H \"Authorization: Bearer [masked]\" https://api.example.com"
        );
        assert_eq!(
            masked("authorization: Basic dXNlcjpwYXNz"),
            "authorization: Basic [masked]"
        );
        assert_eq!(masked("Authorization: abc123"), "Authorization: [masked]");
        assert_eq!(
            masked("Cookie: session=abc123; theme=dark\nnext line"),
            "Cookie: [masked]\nnext line"
        );
        assert_eq!(
            masked("Set-Cookie: id=a3fWa; Expires=Wed"),
            "Set-Cookie: [masked]"
        );
        assert_eq!(masked("X-Api-Key: 0123abcd"), "X-Api-Key: [masked]");
        assert_eq!(
            mask("Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz0123456789").1,
            1
        );
    }

    #[test]
    fn masks_credentials_in_urls() {
        assert_eq!(
            masked("git clone https://deploy:hunter2@git.example.com/acme/shop.git"),
            "git clone https://deploy:[masked]@git.example.com/acme/shop.git"
        );
        assert_eq!(
            masked("postgres://app:s3cr%40t@db.internal:5432/nova"),
            "postgres://app:[masked]@db.internal:5432/nova"
        );
        assert_eq!(
            masked(
                "https://x-access-token:ghs_abcdefghijklmnopqrstuvwxyz0123@github.com/acme/mono"
            ),
            "https://x-access-token:[masked]@github.com/acme/mono"
        );
        assert_eq!(
            masked("remote https://0123456789abcdef0123456789abcdef01234567@github.com/acme/nova"),
            "remote https://[masked]@github.com/acme/nova"
        );
        assert_eq!(mask("redis://:pa55word@cache:6379").1, 1);
    }

    #[test]
    fn masks_private_key_blocks() {
        let text = "key:\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nAAAA\n-----END OPENSSH PRIVATE KEY-----\nafter";
        assert_eq!(masked(text), "key:\n[masked private key]\nafter");
        assert_eq!(
            masked("-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA"),
            "[masked private key]"
        );
        assert_eq!(
            masked("-----BEGIN PRIVATE KEY-----\nAAA\n-----END PRIVATE KEY-----"),
            "[masked private key]"
        );
        assert_eq!(
            masked(
                "-----BEGIN PGP PRIVATE KEY BLOCK-----\nxcMG\n-----END PGP PRIVATE KEY BLOCK-----"
            ),
            "[masked private key]"
        );
    }

    #[test]
    fn masks_secret_assignments() {
        let cases = [
            ("export DB_PASSWORD=hunter2", "export DB_PASSWORD=[masked]"),
            (
                "PGPASSWORD=s3cret psql -h db",
                "PGPASSWORD=[masked] psql -h db",
            ),
            (
                "clientsecret=abc123 dbpassword=xyz",
                "clientsecret=[masked] dbpassword=[masked]",
            ),
            (
                "mysql --password=hunter2 nova",
                "mysql --password=[masked] nova",
            ),
            (
                "GITHUB_TOKEN=abc123 gh pr list",
                "GITHUB_TOKEN=[masked] gh pr list",
            ),
            ("authtoken=abc&user=me", "authtoken=[masked]&user=me"),
            ("api_key = \"abc 123\"", "api_key = \"[masked]\""),
            (
                "AWS_SECRET_ACCESS_KEY='wJalrXUtnFEMI/K7MDENG'",
                "AWS_SECRET_ACCESS_KEY='[masked]'",
            ),
            ("stripe.apiKey=pk_abc", "stripe.apiKey=[masked]"),
            ("passwd=x", "passwd=[masked]"),
            (
                "SSH_PASSPHRASE=open sesame",
                "SSH_PASSPHRASE=[masked] sesame",
            ),
            (
                "?access_token=abc123&page=2",
                "?access_token=[masked]&page=2",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(masked(input), expected, "{input}");
        }
        assert_eq!(mask("clientsecret=abc123 dbpassword=xyz").1, 2);
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let texts = [
            "bundle install: Could not find gem 'rails (= 8.1.0)' in locally installed gems",
            "error[E0425]: cannot find value `token` in this scope",
            "password authentication failed for user \"nova\"",
            "the token expired; run gh auth login again",
            "Authorization failed (HTTP 401)",
            "/Users/someone/Dev/acme/app/models/secret_store.rb:42: syntax error",
            "~/.config/acme/credentials.json not found",
            "git push origin main: ! [rejected] main -> main (fetch first)",
            "commit 3f2a9c1e5b7d4f6a8c0e2b4d6f8a0c2e4b6d8f0a broke the build",
            "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            "uuid 123e4567-e89b-12d3-a456-426614174000 not found",
            "ssh://git@github.com/acme/shop.git and git@github.com:acme/nova.git",
            "https://user@example.com/path and https://example.com:8443/x@y",
            "max_tokens=4000 tokenizer=porter num_tokens=12",
            "password_file=/run/secrets/db secret_name=nova-db SECRET_ID=12",
            "DB_PASSWORD=$DB_PASSWORD TOKEN=${TOKEN} password=<your password>",
            "task-manager-0123456789-abcdef runs disk-image-2024-abcdefghij",
            "use sk-learn or sk-something-quite-long-here",
            "PWD=/Users/me/Dev/acme OLDPWD=/tmp",
            "key_id=abc expires=2026-12-01 keychain=login",
            "Set the authorization header in config/initializers/auth.rb",
            "ghp_short and AKIA123",
            "-----BEGIN PUBLIC KEY-----\nMIIBIjAN\n-----END PUBLIC KEY-----",
            "[masked] stays [masked]",
        ];
        for text in texts {
            unchanged(text);
        }
    }

    #[test]
    fn masking_is_idempotent() {
        let text = "Authorization: Bearer abc\nDB_PASSWORD=hunter2 https://u:p@h ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let (once, count) = mask(text);
        assert_eq!(count, 4);
        assert_eq!(mask(&once), (once.clone(), 0));
    }
}
