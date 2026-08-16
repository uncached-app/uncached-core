//! Paths the user has told us to leave alone.
//!
//! Every other decision in this crate is ours: which directories are
//! artifacts, how sure we are, what deleting one costs. This is the one place
//! where the user overrules all of it. A rule here is not weighed against
//! anything — it wins, whatever the confidence, whatever the provider, whether
//! the cleanup was asked for by hand or by a schedule.
//!
//! # Why this is not the `.gitignore` matcher in [`crate::confidence`]
//!
//! That matcher answers a corroborating question — *does the project consider
//! this disposable?* — so it can afford to be partial. A pattern it cannot
//! parse returns "not established", which costs a candidate some supporting
//! evidence and nothing else.
//!
//! A rule here answers a protective question, and the same partiality would be
//! a defect. If a pattern silently fails to parse, something the user asked us
//! to protect is left unprotected, and the failure is invisible until the day
//! it matters. So this module is exhaustive where the other is conservative:
//! every line either compiles or is reported back with the reason, and none is
//! ever quietly dropped.
//!
//! # Syntax
//!
//! `.gitignore`, with one substitution: patterns anchor to the filesystem root
//! rather than to a repository root. That keeps every familiar rule intact and
//! makes a pasted absolute path work as itself, because an anchored pattern and
//! an absolute path become the same thing.
//!
//! - `node_modules` — any directory with that name, at any depth
//! - `/Users/me/work/vendor` — that path and everything under it
//! - `~/Library/Caches/Homebrew` — the same, with `~` expanded
//! - `**/fixtures/**` — anything under any `fixtures`
//! - `!/Users/me/work/vendor/keep` — an exception, later lines winning

use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

/// A line that could not be turned into a rule, with the reason.
///
/// Returned rather than logged: a rule the user wrote and we discarded has to
/// reach the interface that accepted it, or the list will quietly protect less
/// than it appears to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreRuleError {
    /// 1-based index of the offending line, as the user typed it.
    pub line: usize,
    pub text: String,
    pub reason: String,
}

/// The compiled rules.
///
/// Empty by default, and an empty set matches nothing — a user who has written
/// no rules is protected from this module entirely.
#[derive(Debug, Default)]
pub struct IgnoreRules {
    ignore: GlobSet,
    allow: GlobSet,
    count: usize,
}

impl IgnoreRules {
    /// Compiles rules from the user's list, one per line.
    ///
    /// Returns the rules that compiled alongside the lines that did not, never
    /// one or the other: a single bad line must not disarm the rest of the
    /// list, and must not disappear either.
    pub fn compile(text: &str, home: Option<&Path>) -> (Self, Vec<IgnoreRuleError>) {
        let mut ignore = GlobSetBuilder::new();
        let mut allow = GlobSetBuilder::new();
        let mut errors = Vec::new();
        let mut count = 0;

        for (index, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (negated, pattern) = match line.strip_prefix('!') {
                Some(rest) => (true, rest.trim()),
                None => (false, line),
            };

            match compile_pattern(pattern, home) {
                Ok(globs) => {
                    count += 1;
                    for glob in globs {
                        if negated {
                            allow.add(glob);
                        } else {
                            ignore.add(glob);
                        }
                    }
                }
                Err(reason) => errors.push(IgnoreRuleError {
                    line: index + 1,
                    text: raw.to_owned(),
                    reason,
                }),
            }
        }

        let rules = Self {
            ignore: ignore.build().unwrap_or_default(),
            allow: allow.build().unwrap_or_default(),
            count,
        };
        (rules, errors)
    }

    /// Whether the user has asked us to leave this path alone.
    ///
    /// A negation wins over a match, following `.gitignore`, so a broad rule
    /// can be carved out rather than rewritten.
    pub fn excludes(&self, path: &Path) -> bool {
        if self.count == 0 {
            return false;
        }
        self.ignore.is_match(path) && !self.allow.is_match(path)
    }

    /// How many lines compiled into rules. Comments and blanks do not count.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Turns one `.gitignore` pattern into the globs that implement it.
///
/// Two globs per pattern, not one: a rule naming a directory has to cover the
/// directory itself and everything beneath it, and `foo` does not match
/// `foo/bar`. Missing the second is the failure mode where a rule appears to
/// work — the directory is skipped — right up until something inside it is
/// offered for deletion.
fn compile_pattern(pattern: &str, home: Option<&Path>) -> Result<Vec<Glob>, String> {
    let pattern = pattern.trim_end_matches('/');
    if pattern.is_empty() {
        return Err("empty pattern".to_owned());
    }

    let expanded = match pattern.strip_prefix("~/") {
        Some(rest) => {
            let home = home.ok_or_else(|| "no home directory to expand ~ against".to_owned())?;
            format!("{}/{rest}", home.to_string_lossy().trim_end_matches('/'))
        }
        None => pattern.to_owned(),
    };

    // A pasted path may travel through a symlink — `/var` is `/private/var` on
    // macOS, and a scan reports the resolved form. Resolving the rule too is
    // what keeps a path the user copied from their own terminal from silently
    // matching nothing. Only plain paths are resolved: a pattern with glob
    // syntax is not a path, and one that does not exist yet stays as typed.
    let expanded = match resolve_plain_path(&expanded) {
        Some(resolved) => resolved,
        None => expanded,
    };

    // A pattern with no separator matches by name at any depth; one with a
    // separator is anchored, and here the anchor is the filesystem root.
    let anchored = expanded.contains('/');
    let base = if anchored {
        expanded.clone()
    } else {
        format!("**/{expanded}")
    };

    [base.clone(), format!("{base}/**")]
        .into_iter()
        .map(|glob| {
            Glob::new(&glob).map_err(|error| {
                // globset's message names its own rewritten glob, which the
                // user never typed. Report what they wrote.
                error.kind().to_string().replace(&glob, pattern)
            })
        })
        .collect()
}

fn resolve_plain_path(pattern: &str) -> Option<String> {
    // Absolute only. A bare name like `target` is a name, not a path, and
    // resolving it would silently anchor the rule to whichever directory the
    // process happens to be running in — turning "any target directory" into
    // "this one".
    if !Path::new(pattern).is_absolute() {
        return None;
    }
    if pattern.contains(['*', '?', '[', ']', '{', '}']) {
        return None;
    }
    let canonical = dunce::canonicalize(pattern).ok()?;
    let text = canonical.to_str()?.to_owned();
    // A resolved path carrying glob syntax of its own would change what the
    // rule means, so keep the literal the user wrote instead.
    (!text.contains(['*', '?', '[', ']', '{', '}'])).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::IgnoreRules;
    use std::path::{Path, PathBuf};

    fn rules(text: &str) -> IgnoreRules {
        let (rules, errors) = IgnoreRules::compile(text, Some(Path::new("/home/user")));
        assert!(errors.is_empty(), "unexpected errors: {errors:?}");
        rules
    }

    #[test]
    fn no_rules_exclude_nothing() {
        let rules = rules("");
        assert!(rules.is_empty());
        assert!(!rules.excludes(Path::new("/anything/at/all")));
    }

    #[test]
    fn a_bare_name_matches_at_any_depth() {
        let rules = rules("vendor");

        assert!(rules.excludes(Path::new("/work/project/vendor")));
        assert!(rules.excludes(Path::new("/work/a/b/c/vendor")));
        assert!(!rules.excludes(Path::new("/work/project/vendored")));
    }

    /// The failure that looks like success: the directory is skipped, and then
    /// something inside it is offered for deletion anyway.
    #[test]
    fn a_rule_covers_everything_under_the_path_it_names() {
        let rules = rules("/work/protected");

        assert!(rules.excludes(Path::new("/work/protected")));
        assert!(rules.excludes(Path::new("/work/protected/node_modules")));
        assert!(rules.excludes(Path::new("/work/protected/deep/nested/target")));
        assert!(!rules.excludes(Path::new("/work/protected-sibling")));
    }

    #[test]
    fn an_absolute_path_is_anchored_and_a_relative_one_is_not() {
        let rules = rules("/work/build");

        assert!(rules.excludes(Path::new("/work/build")));
        assert!(!rules.excludes(Path::new("/elsewhere/work/build")));
        assert!(!rules.excludes(Path::new("/other/build")));
    }

    #[test]
    fn a_tilde_expands_to_the_home_directory() {
        let rules = rules("~/Library/Caches/Homebrew");

        assert!(rules.excludes(Path::new("/home/user/Library/Caches/Homebrew")));
        assert!(rules.excludes(Path::new("/home/user/Library/Caches/Homebrew/downloads")));
        assert!(!rules.excludes(Path::new("/home/other/Library/Caches/Homebrew")));
    }

    /// A path copied from the user's own terminal can travel through a
    /// symlink — `/var` is `/private/var` on macOS — while a scan reports the
    /// resolved form. Left unresolved, such a rule compiles cleanly, counts
    /// towards "2 rules in force" and protects nothing.
    ///
    /// Unix only, and stated as the test not existing elsewhere rather than as
    /// a `return` partway through it. Returning early left every line after it
    /// unreachable on Windows — a compile error the moment that platform is
    /// actually built — and left the fixture directory it had already created
    /// behind, because the cleanup was among the lines it skipped.
    #[cfg(unix)]
    #[test]
    fn a_pasted_path_that_travels_through_a_symlink_still_matches() {
        let real = std::env::temp_dir().join("uncached-ignore-real");
        let link = std::env::temp_dir().join("uncached-ignore-link");
        let _ = std::fs::remove_dir_all(&real);
        let _ = std::fs::remove_file(&link);
        std::fs::create_dir_all(real.join("node_modules")).expect("fixture should be created");
        std::os::unix::fs::symlink(&real, &link).expect("symlink should be created");

        // The rule is written with the link; a scan reports what traversal
        // canonicalized, so that is what it has to match.
        let scanned = real
            .canonicalize()
            .expect("fixture should resolve")
            .join("node_modules");
        let (rules, errors) = IgnoreRules::compile(&link.to_string_lossy(), None);

        assert!(errors.is_empty(), "{errors:?}");
        assert!(
            rules.excludes(&scanned),
            "a rule written through the link must cover the resolved path"
        );

        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir_all(&real);
    }

    /// The mistake the fix above invites: resolving a bare name against the
    /// process's working directory would turn "any vendor directory" into
    /// "the one next to wherever the app was launched from".
    #[test]
    fn a_bare_name_is_never_resolved_against_the_working_directory() {
        let rules = rules("target");

        assert!(rules.excludes(Path::new("/somewhere/else/target")));
        assert!(rules.excludes(Path::new("/another/project/target")));
    }

    #[test]
    fn a_negation_carves_an_exception_out_of_a_broader_rule() {
        let rules = rules("node_modules\n!/work/keep/node_modules");

        assert!(rules.excludes(Path::new("/work/other/node_modules")));
        assert!(!rules.excludes(Path::new("/work/keep/node_modules")));
    }

    #[test]
    fn globs_behave_as_they_do_in_gitignore() {
        let rules = rules("**/fixtures/**");

        assert!(rules.excludes(Path::new("/work/project/fixtures/data")));
        assert!(!rules.excludes(Path::new("/work/project/src")));
    }

    #[test]
    fn comments_and_blank_lines_are_not_rules() {
        let rules = rules("# a note\n\n   \nvendor\n");

        assert_eq!(rules.len(), 1);
        assert!(rules.excludes(Path::new("/work/vendor")));
    }

    #[test]
    fn a_trailing_slash_names_the_same_directory() {
        let rules = rules("/work/protected/");

        assert!(rules.excludes(Path::new("/work/protected")));
        assert!(rules.excludes(Path::new("/work/protected/inside")));
    }

    /// A rule that fails to compile must not take the list down with it, and
    /// must not vanish either. Silently dropping it leaves the user believing
    /// something is protected when it is not.
    #[test]
    fn a_broken_line_is_reported_without_disarming_the_others() {
        let (rules, errors) = IgnoreRules::compile(
            "vendor\n/work/[unclosed\ntarget",
            Some(Path::new("/home/user")),
        );

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 2);
        assert_eq!(errors[0].text, "/work/[unclosed");
        assert!(!errors[0].reason.is_empty());

        assert_eq!(rules.len(), 2);
        assert!(rules.excludes(Path::new("/work/vendor")));
        assert!(rules.excludes(Path::new("/work/target")));
        assert!(!rules.excludes(Path::new("/work/[unclosed")));
    }

    #[test]
    fn a_tilde_without_a_home_is_an_error_rather_than_a_literal() {
        let (rules, errors) = IgnoreRules::compile("~/Caches", None);

        assert_eq!(errors.len(), 1);
        assert!(rules.is_empty());
    }

    #[test]
    fn an_empty_pattern_is_reported() {
        let (_, errors) = IgnoreRules::compile("/", Some(Path::new("/home/user")));

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].line, 1);
    }

    #[test]
    fn rules_are_reported_against_the_paths_a_scan_produces() {
        // Guards the shape of the input rather than the matching: a scan hands
        // over owned absolute paths, not the string literals the tests use.
        let rules = rules("target");
        let path = PathBuf::from("/work/project").join("target");

        assert!(rules.excludes(&path));
    }
}
