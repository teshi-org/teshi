//! Gherkin steps for validation CLI self-bootstrap scenarios.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use teshi_core::gherkin_lang::StepKeywordType;
use teshi_core::{BddStep, parse_feature};

use crate::world::{CommandResult, World};

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
    match step.text.trim() {
        "an isolated Feature validation project" | "一个隔离的 Feature 校验项目" => Ok(()),
        "the project contains a malformed zh-CN Feature"
        | "项目包含一个格式错误的 zh-CN Feature" => world.seed_malformed_zh_feature(),
        "the project contains a valid Feature with prose and step attachments"
        | "项目包含带有描述文本和步骤附件的有效 Feature" => {
            world.seed_valid_feature()
        }
        "the project contains a valid zh-CN Feature" | "项目包含一个有效的 zh-CN Feature" => {
            world.seed_valid_zh_feature()
        }
        "the project contains a Feature with unrecognized executable text"
        | "项目包含一个带有无法识别可执行区域文本的 Feature" => {
            world.seed_unrecognized_feature()
        }
        "the project contains one valid and one invalid Feature"
        | "项目包含一个有效和一个无效的 Feature" => {
            world.seed_valid_feature()?;
            world.seed_invalid_feature()
        }
        "the project contains a warning-only Feature" | "项目包含一个只有警告的 Feature" => {
            world.seed_warning_feature()
        }
        "the project contains a repeated-Given Feature" | "项目包含一个重复 Given 的 Feature" => {
            world.seed_repeated_given_feature()
        }
        "the project contains a valid selected directory and an invalid sibling Feature"
        | "项目包含一个有效的选定目录和一个无效的兄弟 Feature" => {
            world.seed_selected_directory_features()
        }
        "the project contains a warning-only Feature and a live daemon"
        | "项目包含一个只有警告的 Feature 和一个运行中的 daemon" => {
            world.seed_warning_feature()?;
            world.start_daemon()
        }
        "the project contains an invalid Feature and a live daemon"
        | "项目包含一个无效 Feature 和一个运行中的 daemon" => {
            world.seed_invalid_feature()?;
            world.start_daemon()
        }
        "the project has an existing active step and an invalid Feature"
        | "项目已有一个活动步骤并且包含一个无效 Feature" => {
            world.select_valid_step_and_snapshot()?;
            world.seed_invalid_feature()
        }
        "the project contains an invalid Feature" | "项目包含一个无效的 Feature" => {
            world.seed_invalid_feature()
        }
        text => bail!("unimplemented Given step: {text}"),
    }
}

fn when(world: &mut World, step: &BddStep) -> Result<()> {
    match step.text.trim() {
        "the tester runs teshi check for the malformed Feature as JSON"
        | "测试人员以 JSON 运行 teshi check 校验格式错误的 Feature" => {
            let feature = world.malformed_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            Ok(())
        }
        "the tester runs teshi check for the valid Feature as JSON"
        | "测试人员以 JSON 运行 teshi check 校验有效的 Feature" => {
            let feature = world.valid_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            Ok(())
        }
        "the tester runs teshi check for the valid zh-CN Feature as JSON"
        | "测试人员以 JSON 运行 teshi check 校验有效的 zh-CN Feature" => {
            let feature = world.valid_zh_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            Ok(())
        }
        "the tester runs teshi check for that Feature as JSON"
        | "测试人员以 JSON 运行 teshi check 校验该 Feature" => {
            let feature = world.unrecognized_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            Ok(())
        }
        "the tester runs teshi check for the invalid Feature as JSON"
        | "测试人员以 JSON 运行 teshi check 校验无效的 Feature" => {
            let feature = world.invalid_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            Ok(())
        }
        "the tester runs teshi check for all Features as JSON"
        | "测试人员以 JSON 运行 teshi check 校验所有 Feature" => {
            world.run_str(&["check", "--all", "--json"])?;
            Ok(())
        }
        "the tester runs teshi check for the warning-only Feature as JSON"
        | "测试人员以 JSON 运行 teshi check 校验只有警告的 Feature" => {
            let feature = world.warning_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            Ok(())
        }
        "the tester runs teshi check for the repeated-Given Feature as JSON"
        | "测试人员以 JSON 运行 teshi check 校验重复 Given 的 Feature" => {
            let feature = world.repeated_given_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            Ok(())
        }
        "the tester starts the BDD run for the selected directory"
        | "测试人员对选定目录启动 BDD 运行" => world.run_selected_directory(),
        "the tester runs teshi check with conflicting scope options"
        | "测试人员同时使用冲突的范围选项运行 teshi check" => {
            let feature = world.invalid_path();
            world.run_str(&["check", "--feature", feature, "--all"])?;
            Ok(())
        }
        "the tester lists unbound steps for the invalid Feature"
        | "测试人员列出无效 Feature 的未绑定步骤" => {
            let feature = world.invalid_path();
            world.run_str(&["steps", "unbound", "--feature", feature])?;
            Ok(())
        }
        "the tester advances to the next unbound step for the invalid Feature"
        | "测试人员为无效 Feature 选择下一个未绑定步骤" => {
            let feature = world.invalid_path();
            world.run_str(&["steps", "next-unbound", "--feature", feature])?;
            Ok(())
        }
        "the tester starts BDD run for the invalid Feature"
        | "测试人员为无效 Feature 启动 BDD 运行" => {
            let runner = std::env::current_exe().context("locate validation E2E runner")?;
            let args = vec![
                "run".to_string(),
                "--runner-cmd".to_string(),
                runner.to_string_lossy().into_owned(),
                world.invalid_path().to_string(),
            ];
            world.run_with_marker(&args, true)?;
            Ok(())
        }
        "the tester starts browser and WinApp replay for the invalid Feature"
        | "测试人员为无效 Feature 启动 browser 和 WinApp replay" => {
            let feature = world.invalid_path();
            world.run_str(&[
                "browser",
                "replay",
                "--feature",
                feature,
                "--dry-run",
                "--non-interactive",
            ])?;
            let feature = world.invalid_path();
            let _ = world.run_str(&[
                "winapp",
                "replay",
                "--feature",
                feature,
                "--dry-run",
                "--non-interactive",
            ])?;
            Ok(())
        }
        "the tester runs the malformed check twice as JSON"
        | "测试人员以 JSON 重复运行两次格式错误的校验" => {
            let feature = world.malformed_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            let feature = world.malformed_path();
            let _ = world.run_str(&["check", "--feature", feature, "--json"])?;
            Ok(())
        }
        "the tester lists unbound steps for the warning-only Feature through the daemon"
        | "测试人员通过 daemon 列出只有警告 Feature 的未绑定步骤" => {
            let feature = world.warning_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            let feature = world.warning_path();
            world.run_str(&["steps", "unbound", "--feature", feature])?;
            Ok(())
        }
        "the tester lists unbound steps for the invalid Feature through the daemon"
        | "测试人员通过 daemon 列出无效 Feature 的未绑定步骤" => {
            let feature = world.invalid_path();
            world.run_str(&["check", "--feature", feature, "--json"])?;
            let feature = world.invalid_path();
            world.run_str(&["steps", "unbound", "--feature", feature])?;
            Ok(())
        }
        text => bail!("unimplemented When step: {text}"),
    }
}

fn then(world: &mut World, step: &BddStep) -> Result<()> {
    let text = step.text.trim();
    if text == "the command fails with exit code 1" || text == "命令以退出码 1 失败" {
        return assert_status(world.last()?, 1);
    }
    if text == "the command succeeds" || text == "命令成功" {
        return assert_status_success(world.last()?);
    }
    if text == "the command fails with exit code 2" || text == "命令以退出码 2 失败" {
        return assert_status(world.last()?, 2);
    }
    if text == "the JSON report has zero errors" || text == "JSON 报告包含零个错误" {
        let report = world.last()?.json()?;
        if report["summary"]["errors"].as_u64() != Some(0) {
            bail!("expected zero validation errors, got {report}");
        }
        return Ok(());
    }
    if text == "the JSON report contains warning codes \"missing_when\" and \"missing_then\""
        || text == "JSON 报告包含警告代码 \"missing_when\" 和 \"missing_then\""
    {
        let report = world.last()?.json()?;
        for code in ["missing_when", "missing_then"] {
            let diagnostic = find_diagnostic(&report, code)?;
            if diagnostic["severity"].as_str() != Some("warning") {
                bail!("expected {code} to be a warning, got {diagnostic}");
            }
        }
        if report["summary"]["errors"].as_u64() != Some(0) {
            bail!("repeated Given fixture unexpectedly has errors: {report}");
        }
        return Ok(());
    }
    if text == "the selected directory run succeeds without sibling diagnostics"
        || text == "选定目录运行成功且不包含兄弟目录诊断"
    {
        let result = world.last()?;
        assert_status_success(result)?;
        if result.combined().contains("missing_step_separator") {
            bail!(
                "selected directory run leaked a sibling diagnostic: {}",
                result.combined()
            );
        }
        return Ok(());
    }
    if text == "the JSON report contains no unrecognized executable line"
        || text == "JSON 报告不包含无法识别的可执行区域行"
    {
        let report = world.last()?.json()?;
        if report["diagnostics"].as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item["code"].as_str() == Some("unrecognized_executable_line"))
        }) {
            bail!("valid fixture unexpectedly has an unrecognized executable line: {report}");
        }
        return Ok(());
    }
    if let Some(rest) = text
        .strip_prefix("the JSON report contains diagnostic code ")
        .or_else(|| rest_after(text, "JSON 报告包含诊断代码 "))
    {
        let (code, line, column) = parse_code_location(rest)?;
        let report = world.last()?.json()?;
        let diagnostic = find_diagnostic(&report, &code)?;
        if diagnostic["line"].as_u64() != Some(line)
            || diagnostic["column"].as_u64() != Some(column)
        {
            bail!(
                "diagnostic {code:?} location mismatch: expected {line}:{column}, got {}:{}",
                diagnostic["line"],
                diagnostic["column"]
            );
        }
        return Ok(());
    }
    if let Some(rest) = text
        .strip_prefix("the diagnostic suggestion is ")
        .or_else(|| rest_after(text, "诊断建议为 "))
    {
        let expected = unquote(rest)?;
        let report = world.last()?.json()?;
        let diagnostic = find_diagnostic(&report, "missing_step_separator")?;
        if diagnostic["suggestion"].as_str() != Some(expected.as_str()) {
            bail!("expected suggestion {expected:?}, got {diagnostic}");
        }
        return Ok(());
    }
    if let Some(rest) = text
        .strip_prefix("the JSON scope is ")
        .or_else(|| rest_after(text, "JSON 范围为 "))
    {
        let expected: serde_json::Value = serde_json::from_str(rest.trim())
            .with_context(|| format!("parse expected scope {rest:?}"))?;
        let report = world.last()?.json()?;
        if report["scope"] != expected {
            bail!("expected scope {expected}, got {}", report["scope"]);
        }
        return Ok(());
    }
    if text == "the all-scope JSON report contains ordered diagnostics"
        || text == "全部范围 JSON 报告包含有序诊断"
    {
        let report = world.last()?.json()?;
        let diagnostics = report["diagnostics"]
            .as_array()
            .context("all-scope report diagnostics")?;
        if diagnostics.is_empty() {
            bail!("expected an all-scope diagnostic");
        }
        if !diagnostics.windows(2).all(|pair| {
            let left = (
                pair[0]["path"].as_str(),
                pair[0]["line"].as_u64(),
                pair[0]["column"].as_u64(),
            );
            let right = (
                pair[1]["path"].as_str(),
                pair[1]["line"].as_u64(),
                pair[1]["column"].as_u64(),
            );
            left <= right
        }) {
            bail!("diagnostics are not ordered by source location: {diagnostics:?}");
        }
        return Ok(());
    }
    if text == "the warning code scenario_starts_without_given is visible"
        || text == "警告代码 scenario_starts_without_given 可见"
    {
        let report = world.last()?.json()?;
        if !report["diagnostics"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["code"].as_str() == Some("scenario_starts_without_given")
                    && item["severity"].as_str() == Some("warning")
            })
        }) {
            bail!("warning code is missing from report: {report}");
        }
        return Ok(());
    }
    if text == "the daemon warning report is structured and visible"
        || text == "daemon 警告报告结构完整且可见"
    {
        let report = first_json_object(&world.last()?.combined())?;
        if report["summary"]["errors"].as_u64() != Some(0)
            || report["summary"]["warnings"].as_u64() == Some(0)
        {
            bail!("daemon warning report has unexpected summary: {report}");
        }
        find_diagnostic(&report, "scenario_starts_without_given")?;
        let direct = world.previous()?.json()?;
        if direct["scope"] != report["scope"] || direct["diagnostics"] != report["diagnostics"] {
            bail!("direct and daemon warning reports differ\ndirect: {direct}\ndaemon: {report}");
        }
        return Ok(());
    }
    if text == "the daemon error contains a structured validation report"
        || text == "daemon 错误包含结构化校验报告"
    {
        let error = first_json_object(&world.last()?.combined())?;
        if error["error"]["code"].as_str() != Some("feature_validation_failed") {
            bail!("daemon error did not preserve validation code: {error}");
        }
        let report = &error["error"]["report"];
        if report["summary"]["errors"].as_u64().unwrap_or(0) == 0 {
            bail!("daemon error report has no errors: {error}");
        }
        find_diagnostic(report, "missing_step_separator")?;
        let direct = world.previous()?.json()?;
        if direct["scope"] != report["scope"] || direct["diagnostics"] != report["diagnostics"] {
            bail!("direct and daemon error reports differ\ndirect: {direct}\ndaemon: {report}");
        }
        return Ok(());
    }
    if text == "the output explains that scope options conflict" || text == "输出说明范围选项互斥"
    {
        let output = world.last()?.combined();
        if !output.contains("cannot be used with") {
            bail!("scope conflict was not explained: {output}");
        }
        return Ok(());
    }
    if text == "the output contains missing_step_separator validation evidence"
        || text == "输出包含 missing_step_separator 校验证据"
    {
        let output = world.last()?.combined();
        if !output.contains("missing_step_separator") {
            bail!("validation evidence is missing: {output}");
        }
        return Ok(());
    }
    if text == "the binding command did not return a successful empty list"
        || text == "绑定命令没有返回成功的空列表"
    {
        let result = world.last()?;
        if result.status == 0 || result.stdout.trim() == "[]" {
            bail!("invalid binding command looked successful: {result:?}");
        }
        return Ok(());
    }
    if text == "the existing active step remains unchanged" || text == "现有活动步骤保持不变"
    {
        world.active_step_is_unchanged()?;
        return Ok(());
    }
    if text == "the nested runner was not started" || text == "嵌套 runner 没有启动" {
        if !world.runner_marker_is_absent() {
            bail!("nested runner marker exists; validation did not fail closed");
        }
        return Ok(());
    }
    if text == "both replay commands failed before target action"
        || text == "两个 replay 命令都在目标操作前失败"
    {
        let previous = world.previous()?;
        let last = world.last()?;
        for result in [previous, last] {
            if result.status == 0 || !result.combined().contains("missing_step_separator") {
                bail!("replay did not fail with validation evidence: {result:?}");
            }
        }
        return Ok(());
    }
    if text == "the two JSON reports are identical" || text == "两份 JSON 报告完全一致" {
        if world.previous()?.json()? != world.last()?.json()? {
            bail!(
                "repeated validation reports differ\nfirst: {}\nsecond: {}",
                world.previous()?.stdout,
                world.last()?.stdout
            );
        }
        return Ok(());
    }
    bail!("unimplemented Then step: {text}")
}

fn assert_status(result: &CommandResult, expected: i32) -> Result<()> {
    if result.status != expected {
        bail!(
            "expected target exit {expected}, got {}\nstdout:\n{}\nstderr:\n{}",
            result.status,
            result.stdout,
            result.stderr
        );
    }
    Ok(())
}

fn assert_status_success(result: &CommandResult) -> Result<()> {
    assert_status(result, 0)
}

fn rest_after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.strip_prefix(prefix)
}

fn parse_code_location(text: &str) -> Result<(String, u64, u64)> {
    let parts = quoted_parts(text);
    if parts.len() != 1 {
        bail!("expected one diagnostic code, got {parts:?}");
    }
    let (line, column) = if let Some((_, tail)) = text.split_once(" at line ") {
        tail.split_once(" column ")
            .context("missing diagnostic column")?
    } else if let Some((_, tail)) = text.split_once("，位于第 ") {
        tail.split_once(" 行第 ")
            .context("missing Chinese diagnostic column")?
    } else {
        bail!("missing diagnostic location")
    };
    let column = column
        .strip_suffix(" 列")
        .or_else(|| column.strip_suffix(" column"))
        .unwrap_or(column);
    Ok((
        parts[0].clone(),
        line.trim().parse().context("diagnostic line")?,
        column.trim().parse().context("diagnostic column")?,
    ))
}

fn find_diagnostic<'a>(report: &'a serde_json::Value, code: &str) -> Result<&'a serde_json::Value> {
    report["diagnostics"]
        .as_array()
        .context("validation report diagnostics")?
        .iter()
        .find(|diagnostic| diagnostic["code"].as_str() == Some(code))
        .with_context(|| format!("diagnostic {code:?} not found in {report}"))
}

fn first_json_object(text: &str) -> Result<serde_json::Value> {
    for (index, character) in text.char_indices() {
        if character != '{' {
            continue;
        }
        let mut deserializer = serde_json::Deserializer::from_str(&text[index..]);
        if let Ok(value) = serde_json::Value::deserialize(&mut deserializer) {
            return Ok(value);
        }
    }
    bail!("no JSON object found in command output: {text}")
}

fn quoted_parts(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '"' {
            continue;
        }
        let mut value = String::new();
        for next in chars.by_ref() {
            if next == '"' {
                break;
            }
            value.push(next);
        }
        out.push(value);
    }
    out
}

fn unquote(text: &str) -> Result<String> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .map(str::to_string)
        .with_context(|| format!("expected quoted text, got {trimmed:?}"))
}
