//! Scenario-to-executor routing declared as `[[runners]]` in `teshi.toml`.
//!
//! Every scenario must match exactly one rule. Zero or several matches stop the
//! run before any executor starts, so a scenario is never routed by accident.
//! The pre-routing `[runner]` section and `TESHI_RUNNER_*` variables are
//! rejected rather than translated.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Project configuration file that declares runners.
pub const RUNNER_CONFIG_FILE: &str = "teshi.toml";

/// Environment variables of the removed single-runner configuration.
pub const LEGACY_RUNNER_ENV: [&str; 3] =
    ["TESHI_RUNNER_CMD", "TESHI_RUNNER_ARGS", "TESHI_RUNNER_CWD"];

const MIGRATION_EXAMPLE: &str = "[[runners]]\nname = \"my-runner\"\ncmd = \"path/to/runner\"\nargs = []\n[runners.match]\npaths = [\"features/**/*.feature\"]";

/// Executors implemented inside Teshi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinExecutor {
    /// In-process step walker for `@api` and mixed scenarios.
    Dispatch,
    /// `teshi-tui-runner` replaying `.tui.json` terminal bindings.
    Tui,
}

impl BuiltinExecutor {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "dispatch" => Some(Self::Dispatch),
            "tui" => Some(Self::Tui),
            _ => None,
        }
    }

    /// Configuration name of the executor.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Tui => "tui",
        }
    }
}

/// How a rule executes its scenarios.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerKind {
    /// An executor implemented inside Teshi.
    Builtin(BuiltinExecutor),
    /// An external process speaking runner protocol v1.
    External {
        /// Executable to spawn.
        cmd: String,
        /// Arguments passed before the request is written to stdin.
        args: Vec<String>,
        /// Working directory, already resolved against the project root.
        cwd: Option<PathBuf>,
        /// Extra environment variables.
        env: HashMap<String, String>,
    },
}

impl fmt::Display for RunnerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Builtin(builtin) => write!(f, "builtin {}", builtin.as_str()),
            Self::External { cmd, .. } => write!(f, "external {cmd}"),
        }
    }
}

/// One validated `[[runners]]` entry.
#[derive(Debug, Clone)]
pub struct RunnerRule {
    /// Unique rule name shown in diagnostics.
    pub name: String,
    /// Executor for matched scenarios.
    pub kind: RunnerKind,
    /// Normalized tags (no `@`, lowercase); any one must be present.
    tags: Vec<String>,
    /// Feature path globs relative to the project root; any one must match.
    paths: Vec<glob::Pattern>,
}

impl RunnerRule {
    /// A rule matches when every declared criterion (tags, paths) has a hit.
    fn matches(&self, relative_path: Option<&str>, tags: &BTreeSet<String>) -> bool {
        let options = glob::MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };
        let tags_hit = self.tags.is_empty() || self.tags.iter().any(|tag| tags.contains(tag));
        let paths_hit = self.paths.is_empty()
            || relative_path.is_some_and(|path| {
                self.paths
                    .iter()
                    .any(|pattern| pattern.matches_with(path, options))
            });
        tags_hit && paths_hit
    }
}

/// Why `teshi.toml` cannot produce routing rules.
#[derive(Debug, thiserror::Error)]
pub enum RouteConfigError {
    /// The file exists but could not be read.
    #[error("read {}: {source}", path.display())]
    Read {
        /// Configuration file.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The file is not valid TOML or a rule has the wrong shape.
    #[error("parse {}: {message}", path.display())]
    Parse {
        /// Configuration file.
        path: PathBuf,
        /// Parser message.
        message: String,
    },
    /// The removed `[runner]` section is still present.
    #[error("{} uses the removed [runner] section; declare [[runners]] routing rules instead:\n{MIGRATION_EXAMPLE}", path.display())]
    LegacyRunnerSection {
        /// Configuration file.
        path: PathBuf,
    },
    /// A removed `TESHI_RUNNER_*` variable is set.
    #[error(
        "{name} is no longer supported; declare [[runners]] in teshi.toml or pass --runner-cmd"
    )]
    LegacyRunnerEnv {
        /// Variable name.
        name: &'static str,
    },
    /// A rule is structurally valid TOML but semantically wrong.
    #[error("runner rule `{name}`: {reason}")]
    InvalidRule {
        /// Rule name (or its index when unnamed).
        name: String,
        /// What is wrong.
        reason: String,
    },
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    runner: Option<toml::Value>,
    #[serde(default)]
    runners: Vec<RuleToml>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleToml {
    name: String,
    builtin: Option<String>,
    cmd: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    env: Option<HashMap<String, String>>,
    #[serde(rename = "match")]
    matcher: Option<MatchToml>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MatchToml {
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
}

/// Validated routing rules of one project.
#[derive(Debug, Clone, Default)]
pub struct RunRoutes {
    rules: Vec<RunnerRule>,
}

/// Loads `[[runners]]` from `<project_root>/teshi.toml`.
///
/// A missing file yields no rules; planning then reports that no runners are
/// configured, so a project without runners can still be opened.
///
/// # Errors
/// Returns [`RouteConfigError`] for unreadable or malformed files, the removed
/// `[runner]` section or `TESHI_RUNNER_*` variables, and invalid rules.
pub fn load_run_routes(project_root: &Path) -> Result<RunRoutes, RouteConfigError> {
    load_run_routes_with_env(project_root, |name| std::env::var(name).ok())
}

/// [`load_run_routes`] with an injectable environment lookup.
///
/// # Errors
/// Same as [`load_run_routes`].
pub fn load_run_routes_with_env(
    project_root: &Path,
    env: impl Fn(&str) -> Option<String>,
) -> Result<RunRoutes, RouteConfigError> {
    if let Some(name) = LEGACY_RUNNER_ENV
        .into_iter()
        .find(|name| env(name).is_some_and(|value| !value.trim().is_empty()))
    {
        return Err(RouteConfigError::LegacyRunnerEnv { name });
    }
    let path = project_root.join(RUNNER_CONFIG_FILE);
    if !path.is_file() {
        return Ok(RunRoutes::default());
    }
    let raw = std::fs::read_to_string(&path).map_err(|source| RouteConfigError::Read {
        path: path.clone(),
        source,
    })?;
    let file: ConfigFile = toml::from_str(&raw).map_err(|error| RouteConfigError::Parse {
        path: path.clone(),
        message: error.to_string(),
    })?;
    if file.runner.is_some() {
        return Err(RouteConfigError::LegacyRunnerSection { path });
    }
    let mut names = BTreeSet::new();
    let rules = file
        .runners
        .into_iter()
        .map(|rule| {
            if !names.insert(rule.name.clone()) {
                return Err(invalid(&rule.name, "duplicate rule name"));
            }
            build_rule(project_root, rule)
        })
        .collect::<Result<_, _>>()?;
    Ok(RunRoutes { rules })
}

fn invalid(name: &str, reason: impl Into<String>) -> RouteConfigError {
    RouteConfigError::InvalidRule {
        name: name.to_string(),
        reason: reason.into(),
    }
}

fn build_rule(project_root: &Path, rule: RuleToml) -> Result<RunnerRule, RouteConfigError> {
    let name = rule.name.trim().to_string();
    if name.is_empty() {
        return Err(invalid("<unnamed>", "`name` must not be empty"));
    }
    let matcher = rule.matcher.unwrap_or_default();
    // An empty matcher would claim every scenario; require an explicit `**`.
    if matcher.tags.is_empty() && matcher.paths.is_empty() {
        return Err(invalid(
            &name,
            "[runners.match] needs `tags` or `paths` (use paths = [\"**\"] to match everything)",
        ));
    }
    let paths = matcher
        .paths
        .iter()
        .map(|source| {
            glob::Pattern::new(source)
                .map_err(|error| invalid(&name, format!("invalid path glob `{source}`: {error}")))
        })
        .collect::<Result<_, _>>()?;
    let tags = matcher
        .tags
        .iter()
        .map(|tag| teshi_core::normalize_tag(tag))
        .collect();
    let kind = match (rule.builtin, rule.cmd) {
        (Some(_), Some(_)) => {
            return Err(invalid(&name, "set either `builtin` or `cmd`, not both"))
        }
        (None, None) => return Err(invalid(&name, "set `builtin` or `cmd`")),
        (Some(builtin), None) => {
            if rule.args.is_some() || rule.cwd.is_some() || rule.env.is_some() {
                return Err(invalid(
                    &name,
                    "`args`, `cwd` and `env` apply only to `cmd` runners",
                ));
            }
            let builtin = BuiltinExecutor::parse(&builtin).ok_or_else(|| {
                invalid(
                    &name,
                    format!("unknown builtin `{builtin}` (expected dispatch or tui)"),
                )
            })?;
            RunnerKind::Builtin(builtin)
        }
        (None, Some(cmd)) => {
            if cmd.trim().is_empty() {
                return Err(invalid(&name, "`cmd` must not be empty"));
            }
            RunnerKind::External {
                cmd,
                args: rule.args.unwrap_or_default(),
                cwd: rule.cwd.map(|cwd| project_root.join(cwd)),
                env: rule.env.unwrap_or_default(),
            }
        }
    };
    Ok(RunnerRule {
        name,
        kind,
        tags,
        paths,
    })
}

/// A scenario to route.
#[derive(Debug, Clone)]
pub struct RouteRequest {
    /// Case identity, carried through to the plan.
    pub id: String,
    /// Feature file, absolute or relative to the project root.
    pub feature_path: PathBuf,
    /// Scenario name inside the Feature.
    pub scenario: String,
}

/// Routing outcome of one scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RouteDecision {
    /// Exactly one rule matched.
    Routed {
        /// Matching rule.
        runner: String,
    },
    /// No rule matched.
    Unmatched,
    /// More than one rule matched.
    Ambiguous {
        /// All matching rules.
        runners: Vec<String>,
    },
    /// The Feature or scenario could not be read, so tags are unknown.
    Unreadable {
        /// Why the scenario could not be inspected.
        reason: String,
    },
}

/// A scenario, the facts used to route it, and the decision.
#[derive(Debug, Clone, Serialize)]
pub struct CaseRoute {
    /// Case identity.
    pub id: String,
    /// Feature path relative to the project root when inside it.
    pub feature: String,
    /// Scenario name.
    pub scenario: String,
    /// Effective tags (Feature and scenario), normalized.
    pub tags: Vec<String>,
    /// Routing outcome.
    pub decision: RouteDecision,
}

/// Scenarios grouped by executor, in order of first appearance.
#[derive(Debug, Clone)]
pub struct RunPlan {
    /// Batches executed one after another.
    pub batches: Vec<RunBatch>,
}

/// One executor and the cases it runs, in request order.
#[derive(Debug, Clone)]
pub struct RunBatch {
    /// Rule that owns the cases.
    pub rule: RunnerRule,
    /// Case identities.
    pub case_ids: Vec<String>,
}

/// Why a selection cannot be planned.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RoutePlanError {
    /// `teshi.toml` declares no `[[runners]]`.
    #[error("no runners configured; declare [[runners]] in teshi.toml:\n{MIGRATION_EXAMPLE}")]
    NoRunners,
    /// Some scenarios matched zero or several rules.
    #[error("{}", describe_unrouted(.0))]
    Unrouted(Vec<CaseRoute>),
}

fn describe_unrouted(routes: &[CaseRoute]) -> String {
    let mut lines = vec![format!(
        "{} scenario(s) cannot be routed to exactly one runner:",
        routes.len()
    )];
    for route in routes {
        let reason = match &route.decision {
            RouteDecision::Unmatched => format!("no rule matches tags [{}]", route.tags.join(", ")),
            RouteDecision::Ambiguous { runners } => {
                format!("matches several rules: {}", runners.join(", "))
            }
            RouteDecision::Unreadable { reason } => reason.clone(),
            RouteDecision::Routed { .. } => continue,
        };
        lines.push(format!(
            "  {} / {}: {reason}",
            route.feature, route.scenario
        ));
    }
    lines.join("\n")
}

impl RunRoutes {
    /// Declared rules in file order.
    #[must_use]
    pub fn rules(&self) -> &[RunnerRule] {
        &self.rules
    }

    /// Routes each scenario without failing, for `--explain` and planning.
    #[must_use]
    pub fn resolve(&self, project_root: &Path, cases: &[RouteRequest]) -> Vec<CaseRoute> {
        let mut features = HashMap::new();
        cases
            .iter()
            .map(|case| self.resolve_one(project_root, case, &mut features))
            .collect()
    }

    fn resolve_one(
        &self,
        project_root: &Path,
        case: &RouteRequest,
        features: &mut HashMap<PathBuf, Result<teshi_core::BddFeature, String>>,
    ) -> CaseRoute {
        let absolute = if case.feature_path.is_absolute() {
            case.feature_path.clone()
        } else {
            project_root.join(&case.feature_path)
        };
        let relative = relative_to_root(project_root, &absolute);
        let feature = features.entry(absolute.clone()).or_insert_with(|| {
            std::fs::read_to_string(&absolute)
                .map(|source| teshi_core::parse_feature(&source, absolute.clone()))
                .map_err(|error| format!("read {}: {error}", absolute.display()))
        });
        let mut route = CaseRoute {
            id: case.id.clone(),
            feature: relative
                .clone()
                .unwrap_or_else(|| absolute.to_string_lossy().into_owned()),
            scenario: case.scenario.clone(),
            tags: Vec::new(),
            decision: RouteDecision::Unmatched,
        };
        let feature = match feature {
            Ok(feature) => feature,
            Err(reason) => {
                route.decision = RouteDecision::Unreadable {
                    reason: reason.clone(),
                };
                return route;
            }
        };
        let Some(scenario) = feature
            .all_scenarios()
            .into_iter()
            .find(|scenario| scenario.name == case.scenario)
        else {
            route.decision = RouteDecision::Unreadable {
                reason: format!("scenario `{}` not found", case.scenario),
            };
            return route;
        };
        let tags: BTreeSet<String> = feature
            .tags
            .iter()
            .chain(&scenario.tags)
            .map(|tag| teshi_core::normalize_tag(tag))
            .collect();
        let matched: Vec<String> = self
            .rules
            .iter()
            .filter(|rule| rule.matches(relative.as_deref(), &tags))
            .map(|rule| rule.name.clone())
            .collect();
        route.tags = tags.into_iter().collect();
        route.decision = match matched.len() {
            0 => RouteDecision::Unmatched,
            1 => RouteDecision::Routed {
                runner: matched.into_iter().next().unwrap_or_default(),
            },
            _ => RouteDecision::Ambiguous { runners: matched },
        };
        route
    }

    /// Groups scenarios into executor batches.
    ///
    /// # Errors
    /// Returns [`RoutePlanError::NoRunners`] without rules and
    /// [`RoutePlanError::Unrouted`] listing every scenario that does not match
    /// exactly one rule.
    pub fn plan(
        &self,
        project_root: &Path,
        cases: &[RouteRequest],
    ) -> Result<RunPlan, RoutePlanError> {
        if self.rules.is_empty() {
            return Err(RoutePlanError::NoRunners);
        }
        let routes = self.resolve(project_root, cases);
        let unrouted: Vec<CaseRoute> = routes
            .iter()
            .filter(|route| !matches!(route.decision, RouteDecision::Routed { .. }))
            .cloned()
            .collect();
        if !unrouted.is_empty() {
            return Err(RoutePlanError::Unrouted(unrouted));
        }
        let mut batches: Vec<RunBatch> = Vec::new();
        for route in routes {
            let RouteDecision::Routed { runner } = route.decision else {
                continue;
            };
            match batches.iter_mut().find(|batch| batch.rule.name == runner) {
                Some(batch) => batch.case_ids.push(route.id),
                None => {
                    if let Some(rule) = self.rules.iter().find(|rule| rule.name == runner) {
                        batches.push(RunBatch {
                            rule: rule.clone(),
                            case_ids: vec![route.id],
                        });
                    }
                }
            }
        }
        Ok(RunPlan { batches })
    }
}

/// `/`-separated path relative to the project root, or `None` outside it.
fn relative_to_root(project_root: &Path, path: &Path) -> Option<String> {
    let relative = path
        .strip_prefix(project_root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            let root = dunce::canonicalize(project_root).ok()?;
            let path = dunce::canonicalize(path).ok()?;
            path.strip_prefix(root).ok().map(Path::to_path_buf)
        })?;
    Some(
        relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(config: &str, features: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(RUNNER_CONFIG_FILE), config).expect("write config");
        for (path, source) in features {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            std::fs::write(path, source).expect("write feature");
        }
        dir
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn request(id: &str, feature: &str, scenario: &str) -> RouteRequest {
        RouteRequest {
            id: id.into(),
            feature_path: PathBuf::from(feature),
            scenario: scenario.into(),
        }
    }

    const RULES: &str = r#"
[[runners]]
name = "terminal"
builtin = "tui"
[runners.match]
tags = ["@tui"]

[[runners]]
name = "cli"
cmd = "cli-runner"
args = ["--quiet"]
cwd = "work"
[runners.match]
tags = ["cli"]
paths = ["features/cli/**/*.feature"]

[[runners]]
name = "api"
builtin = "dispatch"
[runners.match]
tags = ["@API"]
"#;

    const FEATURES: [(&str, &str); 3] = [
        (
            "features/a.feature",
            "@tui\nFeature: A\n  Scenario: one\n    Given x\n  @api\n  Scenario: two\n    Given y\n",
        ),
        (
            "features/cli/b.feature",
            "Feature: B\n  @cli\n  Scenario: three\n    Given z\n",
        ),
        (
            "features/c.feature",
            "Feature: C\n  @cli\n  Scenario: four\n    Given w\n",
        ),
    ];

    #[test]
    fn test_path_globs_match_root_level_and_nested_features() {
        let cases = [
            ("**", "sample.feature", true),
            ("**", "features/deep/a.feature", true),
            ("**/*.feature", "sample.feature", true),
            ("**/*.feature", "features/deep/a.feature", true),
            ("*.feature", "features/a.feature", false),
            ("features/*.feature", "features/deep/a.feature", false),
        ];
        for (glob, path, expected) in cases {
            let config = format!(
                "[[runners]]\nname = \"r\"\nbuiltin = \"tui\"\n[runners.match]\npaths = [\"{glob}\"]\n"
            );
            let dir = project(
                &config,
                &[(path, "Feature: F\n  Scenario: s\n    Given x\n")],
            );
            let routes = load_run_routes_with_env(dir.path(), no_env).expect("routes");
            let resolved = routes.resolve(dir.path(), &[request("s", path, "s")]);
            assert_eq!(
                matches!(resolved[0].decision, RouteDecision::Routed { .. }),
                expected,
                "glob `{glob}` against `{path}`: {:?}",
                resolved[0].decision
            );
        }
    }

    #[test]
    fn test_resolve_routes_by_effective_tags_and_paths() {
        let dir = project(RULES, &FEATURES);
        let routes = load_run_routes_with_env(dir.path(), no_env).expect("routes");
        let resolved = routes.resolve(
            dir.path(),
            &[
                request("a1", "features/a.feature", "one"),
                request("a2", "features/a.feature", "two"),
                request("b", "features/cli/b.feature", "three"),
                request("c", "features/c.feature", "four"),
                request("x", "features/missing.feature", "gone"),
                request("y", "features/a.feature", "absent"),
            ],
        );
        let decisions: Vec<_> = resolved.iter().map(|r| r.decision.clone()).collect();
        assert_eq!(
            decisions[0],
            RouteDecision::Routed {
                runner: "terminal".into()
            },
            "feature tag routes scenario one"
        );
        assert_eq!(
            decisions[1],
            RouteDecision::Ambiguous {
                runners: vec!["terminal".into(), "api".into()]
            },
            "feature @tui plus scenario @api is ambiguous"
        );
        assert_eq!(
            decisions[2],
            RouteDecision::Routed {
                runner: "cli".into()
            }
        );
        assert_eq!(
            decisions[3],
            RouteDecision::Unmatched,
            "tag alone is not enough when the rule also restricts paths"
        );
        assert!(matches!(decisions[4], RouteDecision::Unreadable { .. }));
        assert!(matches!(decisions[5], RouteDecision::Unreadable { .. }));
        assert_eq!(resolved[2].feature, "features/cli/b.feature");
        assert_eq!(resolved[2].tags, vec!["cli"]);
    }

    #[test]
    fn test_plan_groups_batches_in_first_appearance_order() {
        let config = format!(
            "{RULES}\n[[runners]]\nname = \"rest\"\nbuiltin = \"tui\"\n[runners.match]\npaths = [\"features/c.feature\"]\n"
        );
        let dir = project(&config, &FEATURES);
        let routes = load_run_routes_with_env(dir.path(), no_env).expect("routes");
        let plan = routes
            .plan(
                dir.path(),
                &[
                    request("b", "features/cli/b.feature", "three"),
                    request("a1", "features/a.feature", "one"),
                    request("c", "features/c.feature", "four"),
                ],
            )
            .expect("plan");
        let batches: Vec<_> = plan
            .batches
            .iter()
            .map(|b| (b.rule.name.as_str(), b.case_ids.clone()))
            .collect();
        assert_eq!(
            batches,
            vec![
                ("cli", vec!["b".to_string()]),
                ("terminal", vec!["a1".to_string()]),
                ("rest", vec!["c".to_string()]),
            ]
        );
        let RunnerKind::External { cmd, args, cwd, .. } = &plan.batches[0].rule.kind else {
            panic!("cli rule is external");
        };
        assert_eq!(cmd, "cli-runner");
        assert_eq!(args, &vec!["--quiet".to_string()]);
        assert_eq!(cwd.as_deref(), Some(dir.path().join("work").as_path()));
    }

    #[test]
    fn test_plan_rejects_unrouted_scenarios_and_empty_config() {
        let dir = project(RULES, &FEATURES);
        let routes = load_run_routes_with_env(dir.path(), no_env).expect("routes");
        let error = routes
            .plan(
                dir.path(),
                &[
                    request("a1", "features/a.feature", "one"),
                    request("a2", "features/a.feature", "two"),
                    request("c", "features/c.feature", "four"),
                ],
            )
            .expect_err("ambiguous and unmatched scenarios");
        let RoutePlanError::Unrouted(unrouted) = &error else {
            panic!("expected Unrouted, got {error}");
        };
        assert_eq!(unrouted.len(), 2);
        let message = error.to_string();
        assert!(
            message.contains("matches several rules: terminal, api"),
            "{message}"
        );
        assert!(message.contains("no rule matches tags [cli]"), "{message}");

        let empty = project("[api]\nbase_url = \"x\"\n", &FEATURES);
        let routes = load_run_routes_with_env(empty.path(), no_env).expect("no rules is valid");
        assert!(matches!(
            routes.plan(empty.path(), &[request("a1", "features/a.feature", "one")]),
            Err(RoutePlanError::NoRunners)
        ));
    }

    #[test]
    fn test_load_rejects_legacy_and_invalid_configuration() {
        let cases: [(&str, &str); 8] = [
            ("[runner]\ncmd = \"old\"\n", "removed [runner] section"),
            (
                "[[runners]]\nname = \"x\"\nbuiltin = \"tui\"\n",
                "needs `tags` or `paths`",
            ),
            (
                "[[runners]]\nname = \"x\"\nbuiltin = \"tui\"\ncmd = \"y\"\n[runners.match]\ntags = [\"a\"]\n",
                "either `builtin` or `cmd`",
            ),
            (
                "[[runners]]\nname = \"x\"\n[runners.match]\ntags = [\"a\"]\n",
                "set `builtin` or `cmd`",
            ),
            (
                "[[runners]]\nname = \"x\"\nbuiltin = \"nope\"\n[runners.match]\ntags = [\"a\"]\n",
                "unknown builtin `nope`",
            ),
            (
                "[[runners]]\nname = \"x\"\nbuiltin = \"tui\"\nargs = [\"a\"]\n[runners.match]\ntags = [\"a\"]\n",
                "apply only to `cmd` runners",
            ),
            (
                "[[runners]]\nname = \"x\"\ncmd = \"a\"\n[runners.match]\ntags = [\"a\"]\n[[runners]]\nname = \"x\"\ncmd = \"b\"\n[runners.match]\ntags = [\"b\"]\n",
                "duplicate rule name",
            ),
            (
                "[[runners]]\nname = \"x\"\ncmd = \"a\"\nmode = \"y\"\n[runners.match]\ntags = [\"a\"]\n",
                "unknown field",
            ),
        ];
        for (config, expected) in cases {
            let dir = project(config, &[]);
            let message = load_run_routes_with_env(dir.path(), no_env)
                .expect_err(config)
                .to_string();
            assert!(message.contains(expected), "{config}\n=> {message}");
        }
    }

    #[test]
    fn test_load_rejects_legacy_runner_environment() {
        let dir = project(RULES, &[]);
        let error = load_run_routes_with_env(dir.path(), |name| {
            (name == "TESHI_RUNNER_ARGS").then(|| "--x".to_string())
        })
        .expect_err("legacy env");
        assert!(matches!(
            error,
            RouteConfigError::LegacyRunnerEnv {
                name: "TESHI_RUNNER_ARGS"
            }
        ));
    }
}
