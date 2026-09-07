//! Gherkin step implementations for requirement-library CLI E2E.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use teshi_core::gherkin_lang::StepKeywordType;
use teshi_core::{BddStep, parse_feature};

use crate::world::{CHECKOUT_BODY, World};

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

fn given(world: &mut World, step: &BddStep) -> Result<()> {
    let text = step.text.trim();
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
    unimplemented_step("Given", text)
}

fn when(world: &mut World, step: &BddStep) -> Result<()> {
    let text = step.text.trim();
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
            text.strip_prefix("测试人员将文档 ")
                .and_then(|rest| rest.strip_suffix(" 的正文替换为:"))
        })
    {
        let id = unquote(id)?;
        let body = step
            .doc_string
            .as_deref()
            .context("missing replacement body")?;
        let path = world.store_path().join("replacement.md");
        fs::write(&path, format!("{body}\n")).context("write replacement file")?;
        world.run(&[
            "edit",
            &id,
            "--file",
            path.to_str().context("replacement path")?,
        ])?;
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
    unimplemented_step("When", text)
}

fn then(world: &mut World, step: &BddStep) -> Result<()> {
    let text = step.text.trim();
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
    unimplemented_step("Then", text)
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
    use super::quoted_parts;

    #[test]
    fn quoted_parts_keeps_blank_iteration_name() {
        let parts = quoted_parts(r#"assigns document "doc-12" to iteration "   ""#);
        assert_eq!(parts, ["doc-12", "   "]);
    }
}
