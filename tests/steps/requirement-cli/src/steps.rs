//! Gherkin step implementations for requirement-library CLI E2E.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use teshi_core::authoring::ReviewState;
use teshi_core::gherkin_lang::StepKeywordType;
use teshi_core::{BddStep, parse_feature};

use crate::world::{CHECKOUT_BODY, World, link_needle};

/// Runs Background plus scenario steps for one NDJSON case.
pub fn run_case(teshi: &Path, feature_path: &str, scenario_name: &str) -> Result<()> {
    let content =
        fs::read_to_string(feature_path).with_context(|| format!("read feature {feature_path}"))?;
    let feature = parse_feature(&content, PathBuf::from(feature_path));
    let scenario = feature
        .all_scenarios()
        .into_iter()
        .find(|item| item.name == scenario_name)
        .with_context(|| format!("scenario {scenario_name:?} not found in {feature_path}"))?;

    let mut world = World::new(teshi.to_path_buf())?;
    let mut last_kind = StepKeywordType::Given;
    let background = feature
        .background
        .as_ref()
        .map(|block| block.steps.as_slice())
        .unwrap_or(&[]);
    for step in background.iter().chain(scenario.steps.iter()) {
        let kind = match step.keyword_type {
            StepKeywordType::And | StepKeywordType::But => last_kind,
            other => other,
        };
        last_kind = kind;
        run_step(&mut world, kind, step).with_context(|| {
            format!("{} {} (line {})", step.keyword, step.text, step.line_number)
        })?;
    }
    Ok(())
}

fn run_step(world: &mut World, kind: StepKeywordType, step: &BddStep) -> Result<()> {
    match kind {
        StepKeywordType::Given => given(world, step),
        StepKeywordType::When => when(world, step),
        StepKeywordType::Then => then(world, step),
        StepKeywordType::And | StepKeywordType::But => {
            bail!("conjunction step resolved without a leading Given/When/Then")
        }
    }
}

fn handle_project_given(text: &str) -> Result<bool> {
    if text == "project A owns the sample requirements" || text == "项目 A 关联样例需求" {
        return Ok(true);
    }
    Ok(false)
}

fn handle_project_when(world: &mut World, text: &str) -> Result<bool> {
    let parts = quoted_parts(text);
    if text.starts_with("the tester links document ")
        || (text.starts_with("测试人员将文档 ") && text.contains("关联到项目 "))
    {
        let [document, project] = parts.as_slice() else {
            bail!("expected a document and project")
        };
        let root = world.project_path(project)?;
        let root = root.to_str().context("project path is not UTF-8")?;
        world.run(&["link-project", document, "--project", root, "--json"])?;
        return Ok(true);
    }
    if text.starts_with("the tester unlinks document ") || text.starts_with("测试人员解除文档 ")
    {
        let [document, project] = parts.as_slice() else {
            bail!("expected a document and project")
        };
        let root = world.project_path(project)?;
        let root = root.to_str().context("project path is not UTF-8")?;
        world.run(&["unlink-project", document, "--project", root, "--json"])?;
        return Ok(true);
    }
    if text.starts_with("the tester records status ") || text.starts_with("测试人员将项目状态设为 ")
    {
        let [document, project, status] = parts.as_slice() else {
            bail!("expected a document, project, and status")
        };
        let root = world.project_path(project)?;
        let root = root.to_str().context("project path is not UTF-8")?;
        world.run(&[
            "set-project-status",
            document,
            "--project",
            root,
            "--status",
            status,
            "--json",
        ])?;
        return Ok(true);
    }
    Ok(false)
}

fn handle_project_then(world: &mut World, text: &str) -> Result<bool> {
    if text == "project B has no listed requirements" || text == "项目 B 未列出需求" {
        let ids = json_list_ids(&world.last()?.stdout)?;
        if !ids.is_empty() {
            bail!("expected project B to be empty, got {ids:?}");
        }
        return Ok(true);
    }
    if text.starts_with("project ") || text.starts_with("项目 ") {
        let parts = quoted_parts(text);
        if text.contains("recorded project status") || text.contains("已分别记录项目状态")
        {
            let [document, project_a, status_a, project_b, status_b] = parts.as_slice() else {
                bail!("expected document and two project/status pairs")
            };
            let index: serde_json::Value =
                serde_json::from_slice(&fs::read(world.store_path().join("_teshi.json"))?)?;
            let a = world.project_path(project_a)?;
            let b = world.project_path(project_b)?;
            let a_id: serde_json::Value =
                serde_json::from_slice(&fs::read(a.join(".teshi/project.json"))?)?;
            let b_id: serde_json::Value =
                serde_json::from_slice(&fs::read(b.join(".teshi/project.json"))?)?;
            let meta = index["documents"]
                .as_array()
                .context("missing document index")?
                .iter()
                .find(|item| item["id"] == *document)
                .context("document not indexed")?;
            if meta["projects"][a_id["id"].as_str().context("missing A identity")?] != *status_a
                || meta["projects"][b_id["id"].as_str().context("missing B identity")?] != *status_b
            {
                bail!(
                    "project states differ from expectations: {}",
                    meta["projects"]
                );
            }
            return Ok(true);
        }
        if text.contains("only lists the removed document")
            || text.contains("只列出已解除关联的文档")
        {
            let [document] = parts.as_slice() else {
                bail!("expected a document ID")
            };
            let ids = json_list_ids(&world.last()?.stdout)?;
            if ids != [document.to_string()].into_iter().collect() {
                bail!("expected only {document}, got {ids:?}");
            }
            return Ok(true);
        }
        if text.contains("snapshot contains every project source")
            || text.contains("快照包含该项目的所有需求正文")
        {
            let [project] = parts.as_slice() else {
                bail!("expected a project")
            };
            let root = world.project_path(project)?;
            let snapshot: serde_json::Value =
                serde_json::from_slice(&fs::read(root.join("requirements-snapshot.json"))?)?;
            let docs = snapshot["documents"]
                .as_array()
                .context("snapshot has no documents")?;
            if docs.len() != 3
                || !docs.iter().any(|doc| {
                    doc["meta"]["id"] == "doc-12"
                        && doc["body"]
                            .as_str()
                            .is_some_and(|body| body.starts_with("# Login"))
                })
            {
                bail!("project snapshot does not contain the complete linked sources");
            }
            return Ok(true);
        }
    }
    Ok(false)
}

fn given(world: &mut World, step: &BddStep) -> Result<()> {
    let text = step.text.trim();
    if handle_project_given(text)? {
        return Ok(());
    }
    if text == "an isolated requirement library with sample login and checkout documents"
        || text == "已有一份包含登录和结账样例文档的隔离需求库"
    {
        return world.seed_sample();
    }
    if text.contains("belong to iteration") || text.contains("属于迭代") {
        let parts = quoted_parts(text);
        let iteration = parts.last().context("missing iteration name")?.clone();
        let refs: Vec<&str> = parts[..parts.len() - 1]
            .iter()
            .map(String::as_str)
            .collect();
        let result = world.run(&iteration_args(&refs, Some(&iteration), false))?;
        if result.status != 0 {
            bail!(
                "setup assignment failed (exit {}): {}{}",
                result.status,
                result.stderr,
                result.stdout
            );
        }
        return Ok(());
    }
    // Order matters inside `test_point_setup`: the zh append wording
    // ("还链接到文档") contains the plain form ("链接到文档") as a substring.
    if let Some(setup) = test_point_setup(text) {
        let parts = quoted_parts(text);
        let test_point = parts.first().context("missing test point id")?.clone();
        match setup {
            TestPointSetup::SetLinks => {
                let document = parts.get(1).context("missing document id")?.clone();
                world.seed_test_point(&test_point, &[document.as_str()])?;
            }
            TestPointSetup::NoLinks => world.seed_test_point(&test_point, &[])?,
            TestPointSetup::AddLink => {
                let document = parts.get(1).context("missing document id")?;
                world.add_test_point_link(&test_point, document)?;
            }
            TestPointSetup::Approve => {
                world.set_test_point_review_state(&test_point, ReviewState::Approved)?;
            }
        }
        return Ok(());
    }
    unimplemented_step("Given", text)
}

/// Fixture mutation a test-point `Given` step asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestPointSetup {
    /// Replace the test point's links with the single document named in the step.
    SetLinks,
    /// Create the test point with no links.
    NoLinks,
    /// Append a link to the test point named in the step.
    AddLink,
    /// Mark the test point as approved, standing in for human review.
    Approve,
}

fn test_point_setup(text: &str) -> Option<TestPointSetup> {
    if text.contains("also links to document") || text.contains("还链接到文档") {
        return Some(TestPointSetup::AddLink);
    }
    if text.contains("has no links") || text.contains("没有链接") {
        return Some(TestPointSetup::NoLinks);
    }
    if text.contains("linked to document") || text.contains("链接到文档") {
        return Some(TestPointSetup::SetLinks);
    }
    if text.contains("is approved") || text.contains("已批准") {
        return Some(TestPointSetup::Approve);
    }
    None
}

fn when(world: &mut World, step: &BddStep) -> Result<()> {
    let text = step.text.trim();
    if handle_project_when(world, text)? {
        return Ok(());
    }
    if let Some(name) = text
        .strip_prefix("the tester lists requirements in iteration ")
        .and_then(|rest| rest.strip_suffix(" together with unassigned"))
        .or_else(|| {
            text.strip_prefix("测试人员同时按迭代 ")
                .and_then(|rest| rest.strip_suffix(" 和未分配列出需求"))
        })
    {
        let iteration = unquote(name)?;
        world.run(&["list", "--iteration", &iteration, "--unassigned"])?;
        return Ok(());
    }
    if text.starts_with("the tester lists requirements for project ")
        || text.starts_with("测试人员列出项目 ")
    {
        let parts = quoted_parts(text);
        let [project] = parts.as_slice() else {
            bail!("expected one project")
        };
        let root = world.project_path(project)?;
        let root = root.to_str().context("project path is not UTF-8")?;
        world.run(&["list", "--project", root, "--json"])?;
        return Ok(());
    }
    if text.starts_with("the tester lists unassigned project requirements as JSON")
        || text == "测试人员以 JSON 列出未关联项目的需求"
    {
        world.run(&["list", "--unassigned-project", "--json"])?;
        return Ok(());
    }
    if text.starts_with("the tester exports a requirements snapshot for project ")
        || text.starts_with("测试人员导出项目 ")
    {
        let parts = quoted_parts(text);
        let [project] = parts.as_slice() else {
            bail!("expected one project")
        };
        let root = world.project_path(project)?;
        let output = root.join("requirements-snapshot.json");
        let root = root.to_str().context("project path is not UTF-8")?;
        let output = output.to_str().context("snapshot path is not UTF-8")?;
        world.run(&["export-project", "--project", root, "--output", output])?;
        return Ok(());
    }
    if let Some(name) = text
        .strip_prefix("the tester lists requirements in iteration ")
        .and_then(|rest| rest.strip_suffix(" as JSON"))
        .or_else(|| {
            text.strip_prefix("测试人员以 JSON 列出迭代 ")
                .and_then(|rest| rest.strip_suffix(" 中的需求"))
        })
    {
        let iteration = unquote(name)?;
        world.run(&["list", "--iteration", &iteration, "--json"])?;
        return Ok(());
    }
    if text == "the tester lists unassigned requirements as JSON"
        || text == "测试人员以 JSON 列出未分配的需求"
    {
        world.run(&["list", "--unassigned", "--json"])?;
        return Ok(());
    }
    if let Some(title) = text
        .strip_prefix("the tester shows the document titled ")
        .and_then(|rest| rest.strip_suffix(" as JSON"))
        .or_else(|| {
            text.strip_prefix("测试人员以 JSON 显示标题为 ")
                .and_then(|rest| rest.strip_suffix(" 的文档"))
        })
    {
        let title = unquote(title)?;
        world.run(&["show", &title, "--json"])?;
        return Ok(());
    }
    if let Some(title) = text
        .strip_prefix("the tester shows the document titled ")
        .or_else(|| {
            text.strip_prefix("测试人员显示标题为 ")
                .and_then(|rest| rest.strip_suffix(" 的文档"))
        })
    {
        let title = unquote(title)?;
        world.run(&["show", &title])?;
        return Ok(());
    }
    if let Some(id) = text
        .strip_prefix("the tester shows document ")
        .and_then(|rest| rest.strip_suffix(" as JSON"))
        .or_else(|| text.strip_prefix("测试人员以 JSON 显示文档 "))
    {
        let id = unquote(id)?;
        world.run(&["show", &id, "--json"])?;
        return Ok(());
    }
    if let Some(path) = text
        .strip_prefix("the tester shows the document at path ")
        .or_else(|| {
            text.strip_prefix("测试人员显示路径为 ")
                .and_then(|rest| rest.strip_suffix(" 的文档"))
        })
    {
        let path = unquote(path)?;
        world.run(&["show", &path])?;
        return Ok(());
    }
    if let Some(id) = text
        .strip_prefix("the tester shows document ")
        .or_else(|| text.strip_prefix("测试人员显示文档 "))
    {
        let id = unquote(id)?;
        world.run(&["show", &id])?;
        return Ok(());
    }
    if (text.contains("assigns document") && text.contains("to iteration"))
        || text.contains("分配到迭代")
    {
        let parts = quoted_parts(text);
        let iteration = parts.last().context("missing iteration name")?.clone();
        let refs: Vec<&str> = parts[..parts.len() - 1]
            .iter()
            .map(String::as_str)
            .collect();
        let json = wants_json(text);
        world.run(&iteration_args(&refs, Some(&iteration), json))?;
        return Ok(());
    }
    if let Some(rest) = text
        .strip_prefix("the tester clears the iteration on documents ")
        .or_else(|| {
            text.strip_prefix("测试人员清除文档 ")
                .and_then(|rest| rest.strip_suffix(" 的迭代"))
        })
    {
        let refs = quoted_parts(rest);
        let refs: Vec<&str> = refs.iter().map(String::as_str).collect();
        world.run(&iteration_args(&refs, None, false))?;
        return Ok(());
    }
    if let Some(id) = text
        .strip_prefix("the tester replaces the body of document ")
        .and_then(|rest| rest.strip_suffix(" with:"))
        .or_else(|| {
            text.strip_prefix("测试人员")
                .map(|rest| rest.strip_prefix("以 JSON ").unwrap_or(rest))
                .and_then(|rest| rest.strip_prefix("将文档 "))
                .and_then(|rest| rest.strip_suffix(" 的正文替换为:"))
        })
    {
        // An optional trailing "as JSON" selects the machine-readable error
        // envelope; without it failures go to stderr and leave stdout empty.
        let id = id
            .strip_suffix(" as JSON")
            .or_else(|| id.strip_suffix("以 JSON"))
            .unwrap_or(id);
        let id = unquote(id)?;
        let body = step
            .doc_string
            .as_deref()
            .context("missing replacement body")?;
        let path = world.store_path().join("replacement.md");
        fs::write(&path, format!("{body}\n")).context("write replacement file")?;
        let file = path.to_str().context("replacement path")?;
        let mut args = vec!["edit", id.as_str(), "--file", file];
        if wants_json(text) {
            args.push("--json");
        }
        world.run(&args)?;
        return Ok(());
    }
    if let Some(id) = text
        .strip_prefix("the tester resubmits the current body of document ")
        .or_else(|| {
            text.strip_prefix("测试人员重新提交文档 ")
                .and_then(|rest| rest.strip_suffix(" 的当前正文"))
        })
    {
        let id = unquote(id)?;
        let shown = world.inspect(&["show", &id])?;
        let path = world.store_path().join("same-body.md");
        fs::write(&path, &shown.stdout).context("write unchanged body")?;
        world.run(&[
            "edit",
            &id,
            "--file",
            path.to_str().context("same-body path")?,
        ])?;
        return Ok(());
    }
    if let Some(id) = text
        .strip_prefix("the tester edits document ")
        .and_then(|rest| rest.strip_suffix(" as JSON without providing a body"))
        .or_else(|| text.strip_prefix("测试人员在未提供正文的情况下以 JSON 编辑文档 "))
    {
        let id = unquote(id)?;
        world.run(&["edit", &id, "--json"])?;
        return Ok(());
    }
    if text == "the tester opens teshi without a project path"
        || text == "测试人员在不提供项目路径的情况下打开 teshi"
    {
        world.open_tui_without_project()?;
        return Ok(());
    }
    if text == "the tester lists test points as JSON" || text == "测试人员以 JSON 列出测试点"
    {
        world.run_testpoints(&["list", "--json"])?;
        return Ok(());
    }
    if text.starts_with("the tester adds a link on test point ")
        || text.starts_with("测试人员以 JSON 在测试点 ")
    {
        let parts = quoted_parts(text);
        let test_point = parts.first().context("missing test point id")?;
        let document = parts.get(1).context("missing document id")?;
        let range = parts.get(2).context("missing range")?;
        world.run_testpoints(&[
            "link",
            "add",
            test_point,
            "--document",
            document,
            "--range",
            range,
            "--json",
        ])?;
        return Ok(());
    }
    if text.starts_with("the tester removes the link of test point ")
        || text.starts_with("测试人员以 JSON 删除测试点 ")
    {
        let parts = quoted_parts(text);
        let test_point = parts.first().context("missing test point id")?;
        let document = parts.get(1).context("missing document id")?;
        world.run_testpoints(&[
            "link",
            "remove",
            test_point,
            "--document",
            document,
            "--json",
        ])?;
        return Ok(());
    }
    if text.starts_with("the tester retargets the link of test point ")
        || text.starts_with("测试人员以 JSON 将测试点 ")
    {
        let parts = quoted_parts(text);
        let test_point = parts.first().context("missing test point id")?;
        let document = parts.get(1).context("missing document id")?;
        let range = parts.get(2).context("missing range")?;
        world.run_testpoints(&[
            "link",
            "update",
            test_point,
            "--to-document",
            document,
            "--to-range",
            range,
            "--json",
        ])?;
        return Ok(());
    }
    if text.contains("rewrites document") || text.contains("改写文档") {
        let id = quoted_parts(text)
            .first()
            .context("missing document id")?
            .clone();
        // Dropping the anchored sentence from the body is what stales the link,
        // so the rewrite is derived from the same sample sentence the fixture used.
        let needle = link_needle(&id)?;
        let rewritten = world.seeded_body(&id)?.replace(needle, "");
        let path = world.store_path().join("rewritten.md");
        fs::write(&path, rewritten).context("write rewritten body")?;
        world.run(&[
            "edit",
            &id,
            "--file",
            path.to_str().context("rewritten path")?,
            "--json",
        ])?;
        return Ok(());
    }
    unimplemented_step("When", text)
}

fn then(world: &mut World, step: &BddStep) -> Result<()> {
    let text = step.text.trim();
    if handle_project_then(world, text)? {
        return Ok(());
    }
    if text == "the command succeeds" || text == "命令成功" {
        let last = world.last()?;
        if last.status != 0 {
            bail!(
                "expected exit 0, got {}: {}{}",
                last.status,
                last.stderr,
                last.stdout
            );
        }
        return Ok(());
    }
    if text == "the command fails" || text == "命令失败" {
        let last = world.last()?;
        if last.status == 0 {
            bail!("expected a non-zero exit, got 0: {}", last.stdout);
        }
        return Ok(());
    }
    if let Some(code) = text
        .strip_prefix("the command fails with exit code ")
        .or_else(|| text.strip_prefix("命令以退出码 "))
        .and_then(|rest| rest.strip_suffix(" 失败").or(Some(rest)))
    {
        let expected: i32 = code.trim().parse().context("exit code")?;
        let last = world.last()?;
        if last.status != expected {
            bail!(
                "expected exit {expected}, got {}: {}{}",
                last.status,
                last.stderr,
                last.stdout
            );
        }
        return Ok(());
    }
    if text == "no requirement documents are listed" || text == "未列出任何需求文档" {
        let last = world.last()?;
        for id in ["doc-12", "doc-37", "doc-9"] {
            if last.stdout.contains(id) {
                bail!(
                    "expected no listed documents, stdout contained {id}: {}",
                    last.stdout
                );
            }
        }
        return Ok(());
    }
    if text == "the JSON list is empty" || text == "JSON 列表为空" {
        let ids = json_list_ids(world.last()?.stdout.as_str())?;
        if !ids.is_empty() {
            bail!("expected empty JSON list, got {ids:?}");
        }
        return Ok(());
    }
    if text.contains("JSON list contains only document") || text.contains("JSON 列表仅包含文档")
    {
        let expected: HashSet<String> = quoted_parts(text).into_iter().collect();
        let actual: HashSet<String> = json_list_ids(world.last()?.stdout.as_str())?;
        if actual != expected {
            bail!("expected JSON list {expected:?}, got {actual:?}");
        }
        return Ok(());
    }
    if text == "the output is the checkout document body" || text == "输出是结账文档的正文"
    {
        let last = world.last()?;
        if last.stdout.trim_end() != CHECKOUT_BODY.trim_end() {
            bail!("unexpected show output:\n{}", last.stdout);
        }
        return Ok(());
    }
    if text.contains("JSON show envelope has id") || text.contains("JSON 显示信封的 id 为") {
        let parts = quoted_parts(text);
        if parts.len() != 4 {
            bail!("expected id, title, path, iteration quotes, got {parts:?}");
        }
        let value = json_show(world)?;
        let actual_id = value.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let actual_title = value
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let actual_path = value
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let actual_iteration = value
            .get("iteration")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if actual_id != parts[0]
            || actual_title != parts[1]
            || actual_path != parts[2]
            || actual_iteration != parts[3]
        {
            bail!(
                "JSON show envelope mismatch: id={actual_id:?} title={actual_title:?} path={actual_path:?} iteration={actual_iteration:?}, expected {parts:?}"
            );
        }
        return Ok(());
    }
    if let Some(prefix) = text
        .strip_prefix("the JSON show body starts with ")
        .or_else(|| {
            text.strip_prefix("JSON 显示正文以 ")
                .and_then(|rest| rest.strip_suffix(" 开头"))
        })
    {
        let prefix = unquote(prefix)?;
        let value = json_show(world)?;
        let body = value
            .get("body")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !body.starts_with(&prefix) {
            bail!("JSON show body did not start with {prefix:?}:\n{body}");
        }
        return Ok(());
    }
    if text == "the JSON show envelope includes store_id and revision"
        || text == "JSON 显示信封包含 store_id 和 revision"
    {
        let value = json_show(world)?;
        let store_id = value
            .get("store_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let revision = value
            .get("revision")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if store_id.is_empty() {
            bail!("JSON show envelope missing store_id: {value}");
        }
        if revision.is_empty() {
            bail!("JSON show envelope missing revision: {value}");
        }
        return Ok(());
    }
    if let Some(prefix) = text
        .strip_prefix("the output starts with ")
        .or_else(|| text.strip_prefix("输出以 "))
        .and_then(|rest| rest.strip_suffix(" 开头").or(Some(rest)))
    {
        let prefix = unquote(prefix)?;
        let last = world.last()?;
        if !last.stdout.starts_with(&prefix) {
            bail!("output did not start with {prefix:?}:\n{}", last.stdout);
        }
        return Ok(());
    }
    if let Some(code) = text
        .strip_prefix("the JSON error code is ")
        .or_else(|| text.strip_prefix("JSON 错误码为 "))
    {
        let code = unquote(code)?;
        let last = world.last()?;
        let value: serde_json::Value =
            serde_json::from_str(last.stdout.trim()).context("parse JSON error")?;
        let actual = value
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if actual != code {
            bail!(
                "expected JSON code {code:?}, got {actual:?} from {}",
                last.stdout
            );
        }
        return Ok(());
    }
    if text.contains("belong to iteration")
        || text.contains("belongs to iteration")
        || text.contains("属于迭代")
    {
        let parts = quoted_parts(text);
        let iteration = parts.last().context("missing iteration")?.clone();
        for id in &parts[..parts.len() - 1] {
            assert_iteration(world, id, Some(&iteration))?;
        }
        return Ok(());
    }
    if text.contains("are unassigned") || text.contains("未分配迭代") {
        for id in quoted_parts(text) {
            assert_iteration(world, &id, None)?;
        }
        return Ok(());
    }
    if let Some(id) = text
        .strip_prefix("the body of document ")
        .and_then(|rest| rest.strip_suffix(" is:"))
        .or_else(|| {
            text.strip_prefix("文档 ")
                .and_then(|rest| rest.strip_suffix(" 的正文为:"))
        })
    {
        let id = unquote(id)?;
        let expected = step
            .doc_string
            .as_deref()
            .context("missing expected body")?;
        let shown = world.inspect(&["show", &id])?;
        if shown.stdout.trim_end() != expected.trim_end() {
            bail!(
                "body of {id} did not match.\nexpected:\n{expected}\nactual:\n{}",
                shown.stdout
            );
        }
        return Ok(());
    }
    if let Some(id) = text
        .strip_prefix("the revision of document ")
        .and_then(|rest| rest.strip_suffix(" is unchanged"))
        .or_else(|| {
            text.strip_prefix("文档 ")
                .and_then(|rest| rest.strip_suffix(" 的修订未改变"))
        })
    {
        let id = unquote(id)?;
        let expected = world.seeded_revision(&id)?.to_string();
        let shown = world.inspect(&["show", &id, "--json"])?;
        let value: serde_json::Value =
            serde_json::from_str(shown.stdout.trim()).context("parse show JSON")?;
        let actual = value
            .get("revision")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if actual != expected {
            bail!("revision of {id} changed from {expected} to {actual}");
        }
        return Ok(());
    }
    if text.contains("Requirements tab lists documents") || text.contains("需求标签页列出文档")
    {
        let expected: HashSet<String> = quoted_parts(text).into_iter().collect();
        let actual: HashSet<String> = world.tab()?.document_ids.iter().cloned().collect();
        if actual != expected {
            bail!("expected Requirements tab documents {expected:?}, got {actual:?}");
        }
        return Ok(());
    }
    if let Some(id) = text
        .strip_prefix("the Requirements tab shows the body of document ")
        .or_else(|| {
            text.strip_prefix("需求标签页显示文档 ")
                .and_then(|rest| rest.strip_suffix(" 的正文"))
        })
    {
        let id = unquote(id)?;
        let expected = world.seeded_body(&id)?.to_string();
        let tab = world.tab()?;
        if tab.selected_document_id.as_deref() != Some(id.as_str()) {
            bail!(
                "expected selected document {id}, got {:?}",
                tab.selected_document_id
            );
        }
        if tab.selected_body.trim_end() != expected.trim_end() {
            bail!("expected body of {id}, got:\n{}", tab.selected_body);
        }
        return Ok(());
    }
    if let Some((id, expected)) = json_test_point_link_count(text) {
        let value = json_test_points(world)?;
        let links = links_of(find_test_point(&value, &id)?);
        if links.len() != expected {
            bail!(
                "test point {id} has {} link(s), expected {expected}: {links:?}",
                links.len()
            );
        }
        return Ok(());
    }
    if text.contains("points at document") || text.contains("的链接指向文档") {
        let parts = quoted_parts(text);
        let id = parts.first().context("missing test point id")?;
        let document = parts.get(1).context("missing document id")?;
        let links = links_of(find_test_point(&json_test_points(world)?, id)?);
        matching_link(&links, document)?;
        return Ok(());
    }
    if text.contains("is resolved") || text.contains("的链接已解析") {
        let parts = quoted_parts(text);
        let id = parts.first().context("missing test point id")?;
        let document = parts.get(1).context("missing document id")?;
        let links = links_of(find_test_point(&json_test_points(world)?, id)?);
        let matched = matching_link(&links, document)?;
        if matched.get("resolution").and_then(|v| v.as_str()) != Some("resolved") {
            bail!("link of {id} to {document} is not resolved: {matched}");
        }
        return Ok(());
    }
    if text.contains("is stale") || text.contains("的链接已陈旧") {
        let id = quoted_parts(text)
            .first()
            .context("missing test point id")?
            .clone();
        let links = links_of(find_test_point(&json_test_points(world)?, &id)?);
        if links.is_empty() {
            bail!("test point {id} has no link to report as stale");
        }
        for link in &links {
            if link.get("resolution").and_then(|v| v.as_str()) != Some("stale") {
                bail!("test point {id} has a non-stale link: {link}");
            }
        }
        return Ok(());
    }
    if text.contains("is in review state") || text.contains("的审核状态为") {
        let parts = quoted_parts(text);
        let id = parts.first().context("missing test point id")?;
        let expected = parts.get(1).context("missing review state")?;
        let value = json_test_points(world)?;
        let actual = find_test_point(&value, id)?
            .get("review_state")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if actual != expected {
            bail!("test point {id} review state is {actual:?}, expected {expected:?}");
        }
        return Ok(());
    }
    unimplemented_step("Then", text)
}

/// Reads the persisted test points back through the CLI.
///
/// Asserting through a fresh `list` rather than the mutation envelope is what
/// makes "nothing was written" observable for the rejected-target scenarios.
fn json_test_points(world: &World) -> Result<serde_json::Value> {
    let listed = world.inspect_testpoints(&["list", "--json"])?;
    serde_json::from_str(listed.stdout.trim()).context("parse test points JSON")
}

fn find_test_point<'a>(
    value: &'a serde_json::Value,
    test_point_id: &str,
) -> Result<&'a serde_json::Value> {
    value
        .get("test_points")
        .and_then(|v| v.as_array())
        .context("test points JSON missing test_points")?
        .iter()
        .find(|point| point.get("id").and_then(|v| v.as_str()) == Some(test_point_id))
        .with_context(|| format!("test point {test_point_id} missing from the list"))
}

fn links_of(test_point: &serde_json::Value) -> Vec<serde_json::Value> {
    test_point
        .get("links")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Requires exactly one link of `test_point` to target `document`.
fn matching_link<'a>(
    links: &'a [serde_json::Value],
    document: &str,
) -> Result<&'a serde_json::Value> {
    let matched: Vec<&serde_json::Value> = links
        .iter()
        .filter(|link| link.get("document_id").and_then(|v| v.as_str()) == Some(document))
        .collect();
    match matched.as_slice() {
        [link] => Ok(link),
        other => bail!(
            "expected exactly one link to {document}, found {}: {other:?}",
            other.len()
        ),
    }
}

/// Extracts the id and expected count from a `has N link(s)` assertion.
fn json_test_point_link_count(text: &str) -> Option<(String, usize)> {
    if !(text.contains("test point") || text.contains("测试点")) {
        return None;
    }
    if !(text.contains("has") || text.contains('有')) {
        return None;
    }
    let id = quoted_parts(text).first()?.clone();
    let count = text
        .split('"')
        .nth(2)?
        .split_whitespace()
        .find_map(|token| token.parse::<usize>().ok())?;
    Some((id, count))
}

fn assert_iteration(world: &World, document_id: &str, expected: Option<&str>) -> Result<()> {
    let listed = world.inspect(&["list", "--json"])?;
    let value: serde_json::Value =
        serde_json::from_str(listed.stdout.trim()).context("parse list JSON")?;
    let documents = value
        .get("documents")
        .and_then(|v| v.as_array())
        .context("list JSON missing documents")?;
    let doc = documents
        .iter()
        .find(|item| item.get("id").and_then(|v| v.as_str()) == Some(document_id))
        .with_context(|| format!("document {document_id} missing from list"))?;
    let actual = doc.get("iteration").and_then(|v| v.as_str());
    if actual != expected {
        bail!("document {document_id} iteration is {actual:?}, expected {expected:?}");
    }
    Ok(())
}

fn json_list_ids(stdout: &str) -> Result<HashSet<String>> {
    let value: serde_json::Value =
        serde_json::from_str(stdout.trim()).context("parse list JSON")?;
    let documents = value
        .get("documents")
        .and_then(|v| v.as_array())
        .context("list JSON missing documents")?;
    Ok(documents
        .iter()
        .filter_map(|doc| doc.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect())
}

fn json_show(world: &World) -> Result<serde_json::Value> {
    serde_json::from_str(world.last()?.stdout.trim()).context("parse show JSON")
}

fn wants_json(text: &str) -> bool {
    text.contains(" as JSON") || text.contains("以 JSON")
}

fn iteration_args<'a>(refs: &'a [&'a str], iteration: Option<&'a str>, json: bool) -> Vec<&'a str> {
    let mut args = Vec::new();
    if iteration.is_some() {
        args.push("set-iteration");
    } else {
        args.push("clear-iteration");
    }
    args.extend_from_slice(refs);
    if let Some(name) = iteration {
        args.push("--iteration");
        args.push(name);
    }
    if json {
        args.push("--json");
    }
    args
}

fn quoted_parts(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut value = String::new();
        for ch in chars.by_ref() {
            if ch == '"' {
                break;
            }
            value.push(ch);
        }
        out.push(value);
    }
    out
}

fn unquote(text: &str) -> Result<String> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .map(str::to_string)
        .with_context(|| format!("expected quoted string, got {trimmed:?}"))
}

fn unimplemented_step(kind: &str, text: &str) -> Result<()> {
    bail!("unimplemented {kind} step: {text}")
}

#[cfg(test)]
mod tests {
    use super::{TestPointSetup, json_test_point_link_count, quoted_parts, test_point_setup};

    #[test]
    fn quoted_parts_keeps_blank_iteration_name() {
        let parts = quoted_parts(r#"assigns document "doc-12" to iteration "   ""#);
        assert_eq!(parts, ["doc-12", "   "]);
    }

    #[test]
    fn test_point_setup_reads_the_append_form_before_the_plain_form() {
        let cases = [
            (
                r#"test point "tp-1" also links to document "doc-9""#,
                TestPointSetup::AddLink,
            ),
            (
                r#"测试点 "tp-1" 还链接到文档 "doc-9""#,
                TestPointSetup::AddLink,
            ),
            (
                r#"an isolated project with a test point "tp-1" linked to document "doc-12""#,
                TestPointSetup::SetLinks,
            ),
            (
                r#"已有一份隔离项目，其中测试点 "tp-1" 链接到文档 "doc-12""#,
                TestPointSetup::SetLinks,
            ),
            (
                r#"an isolated project with a test point "tp-2" that has no links"#,
                TestPointSetup::NoLinks,
            ),
            (
                r#"已有一份隔离项目，其中测试点 "tp-2" 没有链接"#,
                TestPointSetup::NoLinks,
            ),
            (r#"test point "tp-1" is approved"#, TestPointSetup::Approve),
            (r#"测试点 "tp-1" 已批准"#, TestPointSetup::Approve),
        ];
        for (text, expected) in cases {
            assert_eq!(test_point_setup(text), Some(expected), "for {text:?}");
        }
        assert_eq!(
            test_point_setup(
                "an isolated requirement library with sample login and checkout documents"
            ),
            None
        );
    }

    #[test]
    fn link_count_assertion_parses_both_locales() {
        assert_eq!(
            json_test_point_link_count(r#"the JSON test point "tp-1" has 1 link"#),
            Some(("tp-1".to_string(), 1))
        );
        assert_eq!(
            json_test_point_link_count(r#"the JSON test point "tp-2" has 0 links"#),
            Some(("tp-2".to_string(), 0))
        );
        assert_eq!(
            json_test_point_link_count(r#"JSON 测试点 "tp-1" 有 2 条链接"#),
            Some(("tp-1".to_string(), 2))
        );
    }

    #[test]
    fn link_count_assertion_ignores_the_other_test_point_assertions() {
        for text in [
            r#"the JSON test point "tp-1" is in review state "proposed""#,
            r#"the JSON link of test point "tp-1" is stale"#,
            r#"the JSON link of test point "tp-1" to document "doc-9" is resolved"#,
            r#"the JSON link of test point "tp-1" points at document "doc-9""#,
            r#"JSON 测试点 "tp-1" 的审核状态为 "proposed""#,
            r#"JSON 测试点 "tp-1" 的链接已陈旧"#,
            r#"JSON 测试点 "tp-1" 指向文档 "doc-9" 的链接已解析"#,
            r#"the JSON show envelope has id "doc-12""#,
        ] {
            assert_eq!(json_test_point_link_count(text), None, "matched {text:?}");
        }
    }
}
