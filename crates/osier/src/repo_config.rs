//! "Git config": the git settings that change how a repository behaves, each with the value
//! git uses, where it comes from (this repository, global, system, environment) and what it
//! overrides. Read only for now. Opened from a project's sidebar menu as a dialog, or shown
//! as a tab (Settings > Git > Git config tab).

use crate::app_dialog::AppDialogExt as _;
use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::*;
use osier_git::Repository;
use osier_git::config::{ConfigEntry, Scope};

use crate::app::{Osier, Tab};
use crate::test_ids::TestId as _;

/// A setting worth showing. `key` is matched against git's lower-case output: `{branch}` is
/// the current branch, a `*` segment matches any subsection (one row per match), a trailing
/// `*` matches any key with that prefix.
struct Known {
    key: &'static str,
    /// git's behaviour when nothing sets it.
    default: &'static str,
    help: &'static str,
    /// Every value applies (lists), instead of the last one winning.
    multi: bool,
}

const fn k(key: &'static str, default: &'static str, help: &'static str) -> Known {
    Known {
        key,
        default,
        help,
        multi: false,
    }
}

const fn list(key: &'static str, help: &'static str) -> Known {
    Known {
        key,
        default: "not set",
        help,
        multi: true,
    }
}

const GROUPS: &[(&str, &[Known])] = &[
    (
        "Pull, merge and rebase",
        &[
            k(
                "pull.rebase",
                "not set",
                "How a pull joins your commits with the remote's when both have new ones: \
                 true rebases, false merges. When true, even a fast-forward pull goes through \
                 a rebase, which refuses to start while you have uncommitted changes (Osier \
                 then offers Stash and pull).",
            ),
            k(
                "branch.{branch}.rebase",
                "not set",
                "Like pull.rebase, for the current branch only. Wins over pull.rebase.",
            ),
            k(
                "pull.ff",
                "not set",
                "Whether a pull may fast-forward (true), must always make a merge commit \
                 (false) or may only fast-forward (only).",
            ),
            k(
                "merge.ff",
                "true",
                "The same choice for merges: fast-forward when possible, always make a merge \
                 commit (false), or only fast-forward (only).",
            ),
            k(
                "rebase.autostash",
                "false",
                "Set uncommitted changes aside before a rebase starts and put them back when \
                 it ends.",
            ),
            k(
                "merge.autostash",
                "false",
                "Set uncommitted changes aside before a merge starts and put them back when \
                 it ends.",
            ),
            k(
                "rebase.updaterefs",
                "false",
                "During a rebase, also move other branches that point at the commits being \
                 rebased.",
            ),
            k(
                "merge.conflictstyle",
                "merge",
                "How conflict markers look: merge, diff3 (adds the common ancestor) or zdiff3.",
            ),
            k(
                "rerere.enabled",
                "false",
                "Remember how you resolved a conflict and reuse it when the same conflict \
                 comes back.",
            ),
        ],
    ),
    (
        "Push and fetch",
        &[
            k(
                "push.default",
                "simple",
                "Which branch a push sends when none is named. simple pushes the current \
                 branch to the remote branch it is linked to, when the names match.",
            ),
            k(
                "push.autosetupremote",
                "false",
                "Pushing a branch that is not linked to a remote branch yet links it to one \
                 of the same name, instead of stopping.",
            ),
            k(
                "push.followtags",
                "false",
                "Also push annotated tags that point at the commits being pushed.",
            ),
            k(
                "remote.pushdefault",
                "not set",
                "The remote pushes go to when the branch names none.",
            ),
            k(
                "branch.{branch}.remote",
                "not set",
                "The remote the current branch pulls from.",
            ),
            k(
                "branch.{branch}.merge",
                "not set",
                "The remote branch the current branch is linked to (its upstream).",
            ),
            k(
                "branch.{branch}.pushremote",
                "not set",
                "The remote the current branch pushes to, when it differs from the one it \
                 pulls from.",
            ),
            k(
                "fetch.prune",
                "false",
                "On every fetch, delete remote branches that are gone from the remote.",
            ),
            k(
                "fetch.prunetags",
                "false",
                "On every fetch, also delete tags that are gone from the remote.",
            ),
        ],
    ),
    (
        "Identity and signing",
        &[
            k("user.name", "not set", "The author name on new commits."),
            k(
                "user.email",
                "not set",
                "The author email on new commits. Often set per folder with includeIf, so \
                 check where it comes from.",
            ),
            k("commit.gpgsign", "false", "Sign every commit."),
            k("tag.gpgsign", "false", "Sign every annotated tag."),
            k(
                "gpg.format",
                "openpgp",
                "What signs: openpgp (gpg), ssh or x509.",
            ),
            k(
                "user.signingkey",
                "not set",
                "The key used to sign. Without it git picks one from your identity.",
            ),
        ],
    ),
    (
        "Commits and hooks",
        &[
            k(
                "commit.template",
                "not set",
                "A file whose text starts every new commit message in git's editor.",
            ),
            k(
                "core.hookspath",
                ".git/hooks",
                "The folder git runs hooks from. The hooks found there are listed under \
                 Other sources.",
            ),
        ],
    ),
    (
        "Files and ignoring",
        &[
            k(
                "core.excludesfile",
                "~/.config/git/ignore",
                "An ignore file that applies to every repository on this machine, on top of \
                 each repository's .gitignore.",
            ),
            k(
                "core.autocrlf",
                "false",
                "Line ending conversion. true writes CRLF line endings when checking out and \
                 LF when committing; input only converts CRLF to LF when committing.",
            ),
            k(
                "core.eol",
                "native",
                "The line ending written for text files when .gitattributes asks for \
                 conversion.",
            ),
            k(
                "core.filemode",
                "true",
                "Whether a change to a file's executable bit counts as a change.",
            ),
            k(
                "core.ignorecase",
                "false",
                "Treat file names that differ only in case as the same file. Set by git on \
                 case-insensitive file systems.",
            ),
        ],
    ),
    (
        "Branches",
        &[
            k(
                "init.defaultbranch",
                "master",
                "The first branch's name in new repositories.",
            ),
            k(
                "branch.autosetupmerge",
                "true",
                "A new branch made from a remote branch is linked to it.",
            ),
            k(
                "branch.autosetuprebase",
                "never",
                "Which new linked branches get branch.<name>.rebase = true: never, local, \
                 remote or always.",
            ),
        ],
    ),
    (
        "Remotes and credentials",
        &[
            list(
                "remote.*.url",
                "Where each remote is. Fetch, pull and push use these addresses.",
            ),
            list(
                "credential.helper",
                "Programs that supply usernames and passwords. For https GitHub remotes \
                 Osier adds its own helper while it fetches, pulls and pushes with a chosen \
                 account.",
            ),
            list(
                "credential.*.helper",
                "Credential helpers for one site only.",
            ),
            list(
                "url.*.insteadof",
                "Rewrites remote addresses: any address that starts with the value is \
                 fetched from the address in the key instead.",
            ),
            list("url.*.pushinsteadof", "The same rewrite, for pushes only."),
        ],
    ),
    (
        "Submodules and large repositories",
        &[
            k(
                "submodule.recurse",
                "false",
                "Commands that switch, pull or reset also update submodules.",
            ),
            k(
                "core.fsmonitor",
                "false",
                "Use a file system monitor so status does not scan every file.",
            ),
            k(
                "core.untrackedcache",
                "keep",
                "Cache which folders hold untracked files, to speed up status.",
            ),
            k(
                "feature.manyfiles",
                "false",
                "Turn on the settings that help repositories with very many files.",
            ),
            list(
                "filter.lfs.*",
                "Git LFS: large files are stored outside the repository and fetched when \
                 checked out.",
            ),
        ],
    ),
];

/// Everything the panel shows, read in the background.
#[derive(Debug, Clone)]
pub(crate) struct RepoConfig {
    pub entries: Vec<ConfigEntry>,
    pub branch: Option<String>,
    pub workdir: PathBuf,
    pub hooks_dir: PathBuf,
    pub hooks: Vec<String>,
    /// Rules in `.git/info/exclude` (not comments or blank lines).
    pub exclude_rules: usize,
    pub has_gitattributes: bool,
    /// `GIT_*` variables Osier (and so every git it runs) inherited.
    pub env: Vec<(String, String)>,
}

pub(crate) fn load(repo: &Repository) -> Result<RepoConfig, String> {
    let entries = osier_git::config::list_all(repo).map_err(|e| e.to_string())?;
    let branch = repo
        .git(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .ok()
        .and_then(|o| o.stdout_str().ok().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty());
    let (hooks_dir, hooks) = osier_git::config::hooks(repo).map_err(|e| e.to_string())?;
    let exclude_rules = std::fs::read_to_string(repo.common_dir().join("info").join("exclude"))
        .map(|t| {
            t.lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .count()
        })
        .unwrap_or(0);
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(k, _)| k.starts_with("GIT_"))
        .map(|(k, v)| {
            let secret = ["TOKEN", "PASSWORD", "SECRET"]
                .iter()
                .any(|s| k.contains(s));
            (k, if secret { "(hidden)".into() } else { v })
        })
        .collect();
    env.sort();
    Ok(RepoConfig {
        entries,
        branch,
        workdir: repo.workdir().to_path_buf(),
        hooks_dir,
        hooks,
        exclude_rules,
        has_gitattributes: repo.workdir().join(".gitattributes").is_file(),
        env,
    })
}

/// One shown setting: its key and every value set for it, in git's reading order.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Row<'a> {
    pub key: String,
    pub values: Vec<&'a ConfigEntry>,
    pub default: &'static str,
    pub help: &'static str,
    pub multi: bool,
}

impl Row<'_> {
    /// The value git uses: the last one read, for single-valued keys.
    fn effective(&self) -> Option<&ConfigEntry> {
        if self.multi {
            None
        } else {
            self.values.last().copied()
        }
    }
}

fn matches(pattern: &str, key: &str) -> bool {
    if let Some((section, name)) = pattern.split_once(".*.") {
        // `section.<any subsection>.name`
        key.strip_prefix(section)
            .and_then(|r| r.strip_prefix('.'))
            .and_then(|r| r.strip_suffix(name))
            .and_then(|r| r.strip_suffix('.'))
            .is_some_and(|sub| !sub.is_empty())
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        key.starts_with(prefix)
    } else {
        key == pattern
    }
}

/// The groups of known settings, with what is set for each, plus everything else that is
/// set (`Other settings`).
pub(crate) fn rows(config: &RepoConfig) -> Vec<(&'static str, Vec<Row<'_>>)> {
    let mut used = vec![false; config.entries.len()];
    let mut groups = Vec::new();
    for (title, known) in GROUPS {
        let mut out = Vec::new();
        for item in *known {
            let pattern = if item.key.contains("{branch}") {
                match &config.branch {
                    Some(b) => item.key.replace("{branch}", b),
                    None => continue,
                }
            } else {
                item.key.to_string()
            };
            // Matching keys, in first-seen order; one row each.
            let mut keys: Vec<&str> = Vec::new();
            for (ix, e) in config.entries.iter().enumerate() {
                if matches(&pattern, &e.key) {
                    used[ix] = true;
                    if !keys.contains(&e.key.as_str()) {
                        keys.push(&e.key);
                    }
                }
            }
            if keys.is_empty() {
                out.push(Row {
                    key: pattern,
                    values: Vec::new(),
                    default: item.default,
                    help: item.help,
                    multi: item.multi,
                });
                continue;
            }
            for key in keys {
                out.push(Row {
                    key: key.to_string(),
                    values: config.entries.iter().filter(|e| e.key == key).collect(),
                    default: item.default,
                    help: item.help,
                    multi: item.multi,
                });
            }
        }
        groups.push((*title, out));
    }
    let mut others: Vec<Row> = Vec::new();
    for (ix, e) in config.entries.iter().enumerate() {
        if used[ix] {
            continue;
        }
        match others.iter_mut().find(|r| r.key == e.key) {
            Some(row) => row.values.push(e),
            None => others.push(Row {
                key: e.key.clone(),
                values: vec![e],
                default: "",
                help: "",
                // Unknown keys: show every value; the last one usually wins.
                multi: false,
            }),
        }
    }
    groups.push(("Other settings", others));
    groups
}

fn scope_label(scope: &Scope) -> String {
    match scope {
        Scope::System => "System".into(),
        Scope::Global => "Global".into(),
        Scope::Local => "This repository".into(),
        Scope::Worktree => "This worktree".into(),
        Scope::Command => "Environment".into(),
        Scope::Other(s) => s.clone(),
    }
}

/// "This repository · .git/config", "Global · ~/.gitconfig-work".
fn source_label(e: &ConfigEntry) -> String {
    let place = match e.file() {
        Some(f) if Path::new(f).is_absolute() => crate::format::tilde(Path::new(f)),
        Some(f) => f.to_string(),
        None => e.origin.trim_end_matches(':').to_string(),
    };
    format!("{} · {place}", scope_label(&e.scope))
}

fn value_text(e: &ConfigEntry) -> String {
    match &e.value {
        // git's way to drop the helpers set in files read earlier.
        Some(v) if v.is_empty() && e.key.ends_with(".helper") => {
            "(empty: clears the helpers set before it)".into()
        }
        Some(v) if v.is_empty() => "(empty)".into(),
        Some(v) => v.clone(),
        None => "true".into(),
    }
}

/// Config files in effect, in reading order, with how many values each sets.
fn sources(config: &RepoConfig) -> Vec<(String, Option<PathBuf>, usize)> {
    let mut out: Vec<(String, Option<PathBuf>, usize)> = Vec::new();
    for e in &config.entries {
        let label = source_label(e);
        match out.iter_mut().find(|(l, _, _)| *l == label) {
            Some(s) => s.2 += 1,
            None => {
                let path = e.file().map(|f| {
                    let p = Path::new(f);
                    if p.is_absolute() {
                        p.to_path_buf()
                    } else {
                        config.workdir.join(p)
                    }
                });
                out.push((label, path, 1));
            }
        }
    }
    out
}

/// What the panel is showing: which repository, and its config once read.
#[derive(Debug, Clone)]
pub(crate) struct RepoConfigView {
    pub path: PathBuf,
    pub data: Option<Result<RepoConfig, String>>,
}

impl Osier {
    /// Read `path`'s config in the background into `self.repo_config`.
    pub(crate) fn load_repo_config(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.repo_config = Some(RepoConfigView {
            path: path.clone(),
            data: None,
        });
        let runner = self.runner.clone();
        let task = cx.background_spawn({
            let path = path.clone();
            async move {
                Repository::discover(runner, &path)
                    .map_err(|e| e.to_string())
                    .and_then(|repo| load(&repo))
            }
        });
        cx.spawn(async move |this, cx| {
            let data = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(view) = &mut this.repo_config
                    && view.path == path
                {
                    view.data = Some(data);
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// The tab's content follows the active project: read it when it is not loaded yet.
    pub(crate) fn ensure_repo_config(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.active_project().map(|p| p.path.clone()) else {
            return;
        };
        if self.repo_config.as_ref().is_none_or(|v| v.path != path) {
            self.load_repo_config(path, cx);
        }
    }

    /// Settings > Git > Git config tab.
    pub(crate) fn config_tab_enabled(&self) -> bool {
        self.settings.git.config_tab
    }

    /// The tabs in the tab bar, in order.
    pub(crate) fn visible_tabs(&self) -> Vec<Tab> {
        Tab::ALL
            .into_iter()
            .filter(|t| *t != Tab::Config || self.config_tab_enabled())
            .collect()
    }

    /// "Git config" from a project's sidebar menu: the dialog for `path`.
    pub(crate) fn open_repo_config(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_repo_config(path.clone(), cx);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let view = cx.entity();
        let root_focus = self.focus_handle.clone();
        let focus = cx.focus_handle();
        let inner = focus.clone();
        window.open_app_dialog(cx, move |dialog, window, _| {
            let viewport = window.viewport_size();
            let width = (viewport.width * 0.9).min(px(900.));
            let height = (viewport.height * 0.8).min(px(820.));
            let (view, focus, root_focus) = (view.clone(), inner.clone(), root_focus.clone());
            dialog
                .title(format!("Git config: {name}"))
                .w(width)
                .centered()
                .overlay_closable(true)
                .on_close(move |_, window, cx| root_focus.focus(window, cx))
                .content(move |content, _, cx| {
                    let state = view.read(cx).repo_config.clone();
                    let body = repo_config_body(state.as_ref(), view.downgrade(), cx);
                    content.child(
                        div()
                            .w_full()
                            .h(height)
                            .track_focus(&focus)
                            .test_id("repo-config-dialog")
                            .child(body),
                    )
                })
        });
        // Escape and the close button only reach the dialog while focus is inside it.
        window.defer(cx, move |window, cx| focus.focus(window, cx));
    }

    pub(crate) fn render_repo_config_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.active_project().is_none() {
            return self
                .empty_state("No project", "Open a project to see its git config.", cx)
                .into_any_element();
        }
        self.ensure_repo_config(cx);
        div()
            .size_full()
            .test_id("repo-config-tab")
            .child(repo_config_body(
                self.repo_config.as_ref(),
                cx.entity().downgrade(),
                cx,
            ))
            .into_any_element()
    }
}

/// The panel itself, shared by the dialog and the tab. Takes the state rather than reading
/// the view, because the tab renders while the view is being updated.
fn repo_config_body(
    state: Option<&RepoConfigView>,
    weak: WeakEntity<Osier>,
    cx: &App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let border = cx.theme().border;
    let mono = cx.theme().mono_font_family.clone();
    let Some(state) = state else {
        return div().into_any_element();
    };
    let config = match &state.data {
        None => {
            return div()
                .p_4()
                .text_color(muted)
                .child("Reading the git config…")
                .into_any_element();
        }
        Some(Err(e)) => {
            return div()
                .p_4()
                .child(format!("Could not read the git config: {e}"))
                .into_any_element();
        }
        Some(Ok(c)) => c,
    };

    let reload = {
        let (weak, path) = (weak.clone(), state.path.clone());
        Button::new("repo-config-reload")
            .test_id("repo-config-reload")
            .small()
            .ghost()
            .label("Reload")
            .on_click(move |_, _, cx| {
                let path = path.clone();
                let _ = weak.update(cx, |this, cx| this.load_repo_config(path, cx));
            })
    };
    let mut body = v_flex()
        .id("repo-config-scroll")
        .size_full()
        .overflow_y_scrollbar()
        .p_3()
        .gap_4()
        .child(
            h_flex()
                .gap_2()
                .items_start()
                .child(div().flex_1().min_w_0().text_sm().text_color(muted).child(
                    "The git settings that change how this repository behaves: the \
                             value git uses, where it comes from, and what it overrides. Read \
                             only for now; open a file below to change it.",
                ))
                .child(reload),
        );

    for (title, rows) in rows(config) {
        if rows.is_empty() {
            continue;
        }
        let mut group = v_flex().child(
            div()
                .text_sm()
                .font_medium()
                .pb_1()
                .border_b_1()
                .border_color(border)
                .child(title),
        );
        for row in rows {
            group = group.child(render_row(&row, &mono, muted, border));
        }
        body = body.child(group);
    }

    // Config files, hooks, ignore files and environment.
    let mut other = v_flex().gap_1().child(
        div()
            .text_sm()
            .font_medium()
            .pb_1()
            .border_b_1()
            .border_color(border)
            .child("Other sources"),
    );
    for (ix, (label, path, count)) in sources(config).into_iter().enumerate() {
        let line = h_flex()
            .gap_2()
            .text_sm()
            .child(div().flex_1().min_w_0().child(label))
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(crate::format::count(count, "value")),
            );
        other = other.child(match path {
            Some(path) if path.is_file() => {
                let weak = weak.clone();
                line.child(
                    Button::new(("repo-config-open", ix))
                        .test_id(|| format!("repo-config-open:{ix}"))
                        .xsmall()
                        .ghost()
                        .label("Open")
                        .on_click(move |_, window, cx| {
                            let path = path.clone();
                            let _ =
                                weak.update(cx, |this, cx| this.open_externally(&path, window, cx));
                        }),
                )
            }
            _ => line,
        });
    }
    let hooks = if config.hooks.is_empty() {
        "none".to_string()
    } else {
        config.hooks.join(", ")
    };
    other = other
        .child(fact(
            "Hooks",
            format!(
                "{hooks} (in {})",
                match config.hooks_dir.strip_prefix(&config.workdir) {
                    Ok(rel) => rel.display().to_string(),
                    Err(_) => crate::format::tilde(&config.hooks_dir),
                }
            ),
            muted,
        ))
        .child(fact(
            ".git/info/exclude",
            match config.exclude_rules {
                0 => "no rules (ignores for this clone only)".to_string(),
                n => format!(
                    "{} (ignores for this clone only)",
                    crate::format::count(n, "rule")
                ),
            },
            muted,
        ))
        .child(fact(
            ".gitattributes",
            if config.has_gitattributes {
                "present (line endings, diff and merge rules per file type)"
            } else {
                "none"
            }
            .to_string(),
            muted,
        ));
    if config.env.is_empty() {
        other = other.child(fact("Environment", "no GIT_ variables".into(), muted));
    } else {
        for (k, v) in &config.env {
            other = other.child(fact(k, v.clone(), muted));
        }
    }
    body.child(other).into_any_element()
}

fn fact(label: &str, value: String, muted: Hsla) -> Div {
    h_flex()
        .gap_2()
        .text_sm()
        .items_start()
        .child(div().flex_none().w(px(140.)).child(label.to_string()))
        .child(div().flex_1().min_w_0().text_color(muted).child(value))
}

fn render_row(row: &Row, mono: &SharedString, muted: Hsla, border: Hsla) -> Div {
    let key = div()
        .flex_1()
        .min_w_0()
        .font_family(mono.clone())
        .text_sm()
        .child(row.key.clone());
    let mut out = v_flex()
        .py_2()
        .gap_0p5()
        .border_b_1()
        .border_color(border.opacity(0.5));
    if row.values.is_empty() {
        out = out
            .child(
                h_flex().gap_2().child(key).child(
                    div()
                        .font_family(mono.clone())
                        .text_sm()
                        .text_color(muted)
                        .child(row.default),
                ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Not set: git's default"),
            );
    } else if let Some(effective) = row.effective() {
        out = out
            .child(
                h_flex()
                    .gap_2()
                    .child(key)
                    .child(
                        Tag::secondary()
                            .xsmall()
                            .child(scope_label(&effective.scope)),
                    )
                    .child(
                        div()
                            .font_family(mono.clone())
                            .text_sm()
                            .child(value_text(effective)),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(source_label(effective)),
            );
        // Earlier values the effective one overrides.
        for e in &row.values[..row.values.len() - 1] {
            out = out.child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .line_through()
                    .child(format!("{}: {}", source_label(e), value_text(e))),
            );
        }
    } else {
        // Lists: every value applies.
        out = out.child(h_flex().gap_2().child(key));
        for e in &row.values {
            out = out.child(
                h_flex()
                    .gap_2()
                    .text_xs()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(mono.clone())
                            .child(value_text(e)),
                    )
                    .child(div().text_color(muted).child(source_label(e))),
            );
        }
    }
    if !row.help.is_empty() {
        out = out.child(div().text_xs().text_color(muted).child(row.help));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{RepoConfig, matches, rows};
    use osier_git::config::{ConfigEntry, Scope};
    use std::path::PathBuf;

    fn entry(key: &str, value: &str, scope: Scope, origin: &str) -> ConfigEntry {
        ConfigEntry {
            key: key.into(),
            value: Some(value.into()),
            scope,
            origin: origin.into(),
        }
    }

    fn config(entries: Vec<ConfigEntry>) -> RepoConfig {
        RepoConfig {
            entries,
            branch: Some("main".into()),
            workdir: PathBuf::from("/x/repo"),
            hooks_dir: PathBuf::from("/x/repo/.git/hooks"),
            hooks: vec![],
            exclude_rules: 0,
            has_gitattributes: false,
            env: vec![],
        }
    }

    #[test]
    fn patterns() {
        assert!(matches("pull.rebase", "pull.rebase"));
        assert!(!matches("pull.rebase", "pull.rebasex"));
        assert!(matches("url.*.insteadof", "url.git@github.com:.insteadof"));
        assert!(!matches("url.*.insteadof", "url.insteadof"));
        assert!(matches(
            "credential.*.helper",
            "credential.https://github.com.helper"
        ));
        assert!(!matches("credential.*.helper", "credential.helper"));
        assert!(matches("filter.lfs.*", "filter.lfs.smudge"));
        assert!(matches("remote.*.url", "remote.origin.url"));
    }

    #[test]
    fn last_value_wins_and_others_are_listed_as_overridden() {
        let c = config(vec![
            entry("pull.rebase", "false", Scope::Global, "file:/h/.gitconfig"),
            entry("pull.rebase", "true", Scope::Local, "file:.git/config"),
            entry(
                "branch.main.rebase",
                "false",
                Scope::Local,
                "file:.git/config",
            ),
            entry("alias.co", "checkout", Scope::Global, "file:/h/.gitconfig"),
        ]);
        let groups = rows(&c);
        let pull = groups[0].1.iter().find(|r| r.key == "pull.rebase").unwrap();
        assert_eq!(pull.values.len(), 2);
        assert_eq!(pull.effective().unwrap().scope, Scope::Local);
        assert!(
            groups[0].1.iter().any(|r| r.key == "branch.main.rebase"),
            "current branch substituted"
        );
        let unset = groups[0].1.iter().find(|r| r.key == "merge.ff").unwrap();
        assert!(unset.values.is_empty());
        assert_eq!(unset.default, "true");
        let (title, others) = groups.last().unwrap();
        assert_eq!(*title, "Other settings");
        assert_eq!(others.len(), 1, "only what no group shows");
        assert_eq!(others[0].key, "alias.co");
    }

    #[test]
    fn wildcard_rows_one_per_key_and_branch_rows_need_a_branch() {
        let mut c = config(vec![
            entry("remote.origin.url", "a", Scope::Local, "file:.git/config"),
            entry("remote.fork.url", "b", Scope::Local, "file:.git/config"),
        ]);
        c.branch = None;
        let groups = rows(&c);
        let all: Vec<&str> = groups
            .iter()
            .flat_map(|(_, r)| r.iter().map(|r| r.key.as_str()))
            .collect();
        assert!(all.contains(&"remote.origin.url") && all.contains(&"remote.fork.url"));
        assert!(
            !all.iter().any(|k| k.contains("{branch}")),
            "no branch, no row"
        );
    }
}
