//! GitHub API calls through `gh api`, as one chosen account, for "Clone repository…" and
//! "Create new repository…".
//!
//! Like git's credentials, the token is read only in a short-lived helper process that is
//! Osier itself: the app runs `osier --gh-api --account <login> --host <host> -- <args>`
//! ([`run_as`]), which asks gh for the account's token and hands it to the child `gh api`
//! through `GH_TOKEN`. The account is used without `gh auth switch`, and the token never
//! reaches the app, a log, argv or the disk.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::{AuthError, Result};

/// A repository the account can reach.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GhRepo {
    pub full_name: String,
    pub name: String,
    pub owner: String,
    pub clone_url: String,
    pub ssh_url: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_template: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub fork: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct License {
    pub key: String,
    pub name: String,
}

/// What "Create new repository…" asks GitHub for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewRepo {
    /// The account itself or one of its organisations.
    pub owner: String,
    pub name: String,
    pub description: String,
    pub private: bool,
    /// `owner/name` of a template repository. GitHub ignores the starter files with one.
    pub template: Option<String>,
    pub readme: bool,
    pub gitignore: Option<String>,
    pub license: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CreatedRepo {
    pub full_name: String,
    pub clone_url: String,
    pub html_url: String,
}

/// The jq filter that turns a page of `user/repos` into one JSON object per line.
const REPO_FIELDS: &str = ".[] | {full_name, name, owner: .owner.login, clone_url, ssh_url, \
                           private, description, is_template, archived, fork}";

/// Every repository `login` can reach: its own, its organisations' and the ones it
/// collaborates on.
pub fn repos(gh: &Gh) -> Result<Vec<GhRepo>> {
    let out = api(
        gh,
        &[
            "--paginate",
            "--jq",
            REPO_FIELDS,
            "user/repos?per_page=100&sort=full_name&affiliation=owner,collaborator,organization_member",
        ],
        None,
    )?;
    parse_repos(&out)
}

pub fn parse_repos(lines: &str) -> Result<Vec<GhRepo>> {
    lines
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| AuthError::Parse(e.to_string())))
        .collect()
}

/// Template repositories owned by any of `owners` (the account and its organisations), in
/// one search request instead of reading every repository.
pub fn templates(gh: &Gh, owners: &[String]) -> Result<Vec<String>> {
    let path = templates_query(owners)?;
    let out = api(gh, &["--jq", ".items[].full_name", &path], None)?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// The search for [`templates`]: the first owner is the account (a user), the rest are its
/// organisations; GitHub ORs the qualifiers.
pub fn templates_query(owners: &[String]) -> Result<String> {
    let mut q = String::from("template:true");
    for (ix, owner) in owners.iter().enumerate() {
        let owner = path_segment(owner)?;
        q.push_str(&format!(
            "+{}:{owner}",
            if ix == 0 { "user" } else { "org" }
        ));
    }
    Ok(format!("search/repositories?q={q}&per_page=100"))
}

/// The organisations `login` belongs to.
pub fn orgs(gh: &Gh) -> Result<Vec<String>> {
    let out = api(
        gh,
        &["--paginate", "--jq", ".[].login", "user/orgs?per_page=100"],
        None,
    )?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

/// Names of GitHub's .gitignore templates ("Rust", "Node", ...).
pub fn gitignore_templates(gh: &Gh) -> Result<Vec<String>> {
    let out = api(gh, &["gitignore/templates"], None)?;
    serde_json::from_str(&out).map_err(|e| AuthError::Parse(e.to_string()))
}

/// The text of one .gitignore template.
pub fn gitignore_template(gh: &Gh, name: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Template {
        source: String,
    }
    let path = format!("gitignore/templates/{}", path_segment(name)?);
    let out = api(gh, &[&path], None)?;
    let t: Template = serde_json::from_str(&out).map_err(|e| AuthError::Parse(e.to_string()))?;
    Ok(t.source)
}

/// The licences GitHub offers.
pub fn licenses(gh: &Gh) -> Result<Vec<License>> {
    let out = api(gh, &["licenses?per_page=100"], None)?;
    serde_json::from_str(&out).map_err(|e| AuthError::Parse(e.to_string()))
}

/// The text of one licence with `[year]` and `[fullname]` filled in.
pub fn license_text(gh: &Gh, key: &str, year: i32, holder: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Body {
        body: String,
    }
    let path = format!("licenses/{}", path_segment(key)?);
    let out = api(gh, &[&path], None)?;
    let b: Body = serde_json::from_str(&out).map_err(|e| AuthError::Parse(e.to_string()))?;
    Ok(fill_license(&b.body, year, holder))
}

pub fn fill_license(body: &str, year: i32, holder: &str) -> String {
    let mut text = body.replace("[year]", &year.to_string());
    if !holder.trim().is_empty() {
        text = text.replace("[fullname]", holder.trim());
    }
    text
}

/// Create the repository on GitHub and return where to clone it from.
pub fn create(gh: &Gh, new: &NewRepo) -> Result<CreatedRepo> {
    path_segment(&new.name)?;
    path_segment(&new.owner)?;
    let (endpoint, body) = request_for(gh.login, new)?;
    let out = api(
        gh,
        &["--method", "POST", &endpoint, "--input", "-"],
        Some(&body),
    )?;
    let created: CreatedRepo =
        serde_json::from_str(&out).map_err(|e| AuthError::Parse(e.to_string()))?;
    if new.template.is_some() {
        wait_for_content(gh, &created.full_name);
    }
    Ok(created)
}

/// The endpoint and JSON body for `new`, as `login`.
pub fn request_for(login: &str, new: &NewRepo) -> Result<(String, String)> {
    #[derive(Serialize)]
    struct Plain<'a> {
        name: &'a str,
        description: &'a str,
        private: bool,
        auto_init: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        gitignore_template: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        license_template: Option<&'a str>,
    }
    #[derive(Serialize)]
    struct FromTemplate<'a> {
        owner: &'a str,
        name: &'a str,
        description: &'a str,
        private: bool,
        include_all_branches: bool,
    }
    let encode = |v: std::result::Result<String, serde_json::Error>| {
        v.map_err(|e| AuthError::Parse(e.to_string()))
    };
    if let Some(template) = &new.template {
        let (t_owner, t_name) = template
            .split_once('/')
            .ok_or_else(|| AuthError::Parse(format!("template {template} is not owner/name")))?;
        let endpoint = format!(
            "repos/{}/{}/generate",
            path_segment(t_owner)?,
            path_segment(t_name)?
        );
        let body = encode(serde_json::to_string(&FromTemplate {
            owner: &new.owner,
            name: &new.name,
            description: &new.description,
            private: new.private,
            include_all_branches: false,
        }))?;
        return Ok((endpoint, body));
    }
    let endpoint = if new.owner.is_empty() || new.owner.eq_ignore_ascii_case(login) {
        "user/repos".to_string()
    } else {
        format!("orgs/{}/repos", path_segment(&new.owner)?)
    };
    let body = encode(serde_json::to_string(&Plain {
        name: &new.name,
        description: &new.description,
        private: new.private,
        // A licence or .gitignore needs a first commit; GitHub makes it with the README.
        auto_init: new.readme,
        gitignore_template: new.gitignore.as_deref(),
        license_template: new.license.as_deref(),
    }))?;
    Ok((endpoint, body))
}

/// GitHub fills a repository made from a template in the background; wait (a few seconds at
/// most) until it has a commit, so the clone is not empty.
fn wait_for_content(gh: &Gh, full_name: &str) {
    let path = format!("repos/{full_name}/commits?per_page=1");
    for _ in 0..20 {
        if api(gh, &["--silent", &path], None).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

/// A name that is safe as one URL path segment (GitHub's own rules are stricter).
fn path_segment(s: &str) -> Result<&str> {
    let ok = !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c));
    if ok {
        Ok(s)
    } else {
        Err(AuthError::Parse(format!("{s:?} is not a valid name")))
    }
}

/// One `gh api` call as the app made it, for the command log. Never holds the token.
#[derive(Debug, Clone)]
pub struct ApiCall {
    /// The helper invocation exactly as run: `osier --gh-api --account .. --host .. -- ..`.
    pub argv: Vec<String>,
    pub started: SystemTime,
    pub duration: Duration,
    /// `None` when the helper could not be started.
    pub status: Option<i32>,
    pub stdout_len: usize,
    /// gh's stderr, or GitHub's error message.
    pub stderr: String,
}

/// Who `gh api` runs as, through which Osier binary, and who hears about each call.
#[derive(Clone, Copy)]
pub struct Gh<'a> {
    pub exe: &'a Path,
    pub login: &'a str,
    pub host: &'a str,
    pub observer: Option<&'a (dyn Fn(&ApiCall) + Sync)>,
}

impl std::fmt::Debug for Gh<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gh")
            .field("login", &self.login)
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

/// `gh api <args>` as `gh.login` on `gh.host`, through the `--gh-api` helper, with `stdin`
/// as the request body when given.
fn api(gh: &Gh, args: &[&str], stdin: Option<&str>) -> Result<String> {
    let mut argv: Vec<String> = ["--gh-api", "--account", gh.login, "--host", gh.host, "--"]
        .iter()
        .chain(args)
        .map(|a| a.to_string())
        .collect();
    let started = SystemTime::now();
    let clock = Instant::now();
    let result = run_helper(gh.exe, &argv, stdin);
    if let Some(observe) = gh.observer {
        argv.insert(0, gh.exe.to_string_lossy().into_owned());
        let (status, stdout_len, stderr) = match &result {
            Ok(out) => (Some(0), out.len(), String::new()),
            Err((status, e)) => (*status, 0, e.to_string()),
        };
        observe(&ApiCall {
            argv,
            started,
            duration: clock.elapsed(),
            status,
            stdout_len,
            stderr,
        });
    }
    result.map_err(|(_, e)| e)
}

/// Run the helper; on failure the exit status (if it ran) and the error.
fn run_helper(
    exe: &Path,
    argv: &[String],
    stdin: Option<&str>,
) -> std::result::Result<String, (Option<i32>, AuthError)> {
    let mut cmd = Command::new(exe);
    cmd.args(argv)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| (None, AuthError::Io(e)))?;
    if let (Some(body), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(body.as_bytes())
            .map_err(|e| (None, AuthError::Io(e)))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| (None, AuthError::Io(e)))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        // `gh api` prints GitHub's JSON error on stdout; its message is the useful part
        // (a rate limit, a missing scope, a name that is taken).
        let message = serde_json::from_slice::<serde_json::Value>(&out.stdout)
            .ok()
            .and_then(|v| {
                let main = v.get("message")?.as_str()?.to_string();
                let detail = v
                    .get("errors")
                    .and_then(|e| e.as_array())
                    .and_then(|e| e.first())
                    .and_then(|e| e.get("message"))
                    .and_then(|m| m.as_str())
                    .map(|m| format!(": {m}"))
                    .unwrap_or_default();
                Some(format!("{main}{detail}"))
            });
        return Err((out.status.code(), AuthError::Gh(message.unwrap_or(stderr))));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The `--gh-api` helper: run `gh api --hostname <host> <args>` as `login` with this
/// process's stdin, stdout and stderr. Returns the exit code.
pub fn run_as(login: &str, host: &str, args: &[String]) -> Result<i32> {
    let token = crate::gh::token(login, host)?;
    let status = Command::new("gh")
        .arg("api")
        .args(["--hostname", host])
        .args(args)
        .env("GH_TOKEN", &token)
        .env("GH_ENTERPRISE_TOKEN", &token)
        .env("GH_HOST", host)
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .status()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AuthError::GhMissing
            } else {
                AuthError::Io(e)
            }
        })?;
    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_repo_per_line() {
        let lines = r#"{"full_name":"alice/app","name":"app","owner":"alice","clone_url":"https://github.com/alice/app.git","ssh_url":"git@github.com:alice/app.git","private":true,"description":null,"is_template":false,"archived":false,"fork":false}
{"full_name":"acme/tpl","name":"tpl","owner":"acme","clone_url":"https://github.com/acme/tpl.git","ssh_url":"git@github.com:acme/tpl.git","private":false,"description":"A template","is_template":true,"archived":false,"fork":false}
"#;
        let repos = parse_repos(lines).unwrap();
        assert_eq!(repos.len(), 2);
        assert!(repos[0].private && repos[0].description.is_none());
        assert!(repos[1].is_template);
        assert_eq!(repos[1].description.as_deref(), Some("A template"));
        assert!(parse_repos("not json").is_err());
    }

    #[test]
    fn requests_go_to_the_right_endpoint() {
        let mut new = NewRepo {
            owner: "alice".into(),
            name: "app".into(),
            description: "Hi \"there\"".into(),
            private: true,
            readme: true,
            gitignore: Some("Rust".into()),
            license: Some("mit".into()),
            template: None,
        };
        let (endpoint, body) = request_for("alice", &new).unwrap();
        assert_eq!(endpoint, "user/repos");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["auto_init"], true);
        assert_eq!(v["gitignore_template"], "Rust");
        assert_eq!(v["description"], "Hi \"there\"");

        new.owner = "acme".into();
        assert_eq!(request_for("alice", &new).unwrap().0, "orgs/acme/repos");

        new.template = Some("acme/tpl".into());
        let (endpoint, body) = request_for("alice", &new).unwrap();
        assert_eq!(endpoint, "repos/acme/tpl/generate");
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["owner"], "acme");
        assert!(v.get("auto_init").is_none());

        new.template = Some("../x".into());
        assert!(request_for("alice", &new).is_err());
        assert!(path_segment("a/b").is_err());
        assert!(path_segment("..").is_err());
        assert!(path_segment("my.repo-2").is_ok());
    }

    #[test]
    fn license_placeholders_are_filled() {
        let t = fill_license("Copyright (c) [year] [fullname]\n", 2026, "Ada");
        assert_eq!(t, "Copyright (c) 2026 Ada\n");
        let t = fill_license("Copyright (c) [year] [fullname]\n", 2026, " ");
        assert_eq!(t, "Copyright (c) 2026 [fullname]\n");
    }

    #[test]
    fn templates_are_one_search_over_the_account_and_its_orgs() {
        let q = templates_query(&["alice".into(), "acme".into(), "labs".into()]).unwrap();
        assert_eq!(
            q,
            "search/repositories?q=template:true+user:alice+org:acme+org:labs&per_page=100"
        );
        assert!(templates_query(&["a b".into()]).is_err());
    }

    #[test]
    fn every_call_is_reported_without_the_token() {
        use std::sync::Mutex;
        let seen: Mutex<Vec<ApiCall>> = Mutex::new(Vec::new());
        let observe = |c: &ApiCall| seen.lock().unwrap().push(c.clone());
        // Stand-ins for the Osier binary: one that succeeds, one that fails.
        for (exe, ok) in [("/bin/true", true), ("/bin/false", false)] {
            let gh = Gh {
                exe: Path::new(exe),
                login: "alice",
                host: "github.com",
                observer: Some(&observe),
            };
            assert_eq!(api(&gh, &["user"], None).is_ok(), ok);
        }
        let seen = seen.into_inner().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen[0].argv,
            [
                "/bin/true",
                "--gh-api",
                "--account",
                "alice",
                "--host",
                "github.com",
                "--",
                "user"
            ]
        );
        assert_eq!(seen[0].status, Some(0));
        assert_eq!(seen[1].status, Some(1));
    }
}
