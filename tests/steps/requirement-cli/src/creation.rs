//! Real process coverage for requirement creation in both Feature locales.

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};
use teshi_core::BddStep;

/// Executes creation steps against child CLI processes and an isolated store.
///
/// # Errors
/// Returns failed assertions, unsupported steps, process errors, or fixture I/O failures.
pub(crate) fn run(teshi: &Path, steps: &[BddStep]) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let store = root.join("store");
    let project = root.join("project");
    fs::create_dir_all(&store)?;
    fs::create_dir_all(&project)?;
    let command = || {
        let mut cmd = Command::new(teshi);
        cmd.args(["requirements", "--requirements-root"])
            .arg(&store)
            .current_dir(&project)
            .env("TESHI_APP_DATA_DIR", root.join("app-data"))
            .env("TESHI_SESSION_DIR", root.join("sessions"))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    };
    let create = |title: &str| -> Result<Output> {
        Ok(command()
            .args(["create", "--title", title, "--json"])
            .output()?)
    };
    let mut outputs = Vec::new();
    let mut reserved = String::new();
    let mut before = Vec::new();
    for step in steps {
        let text = step.text.as_str();
        match text {
            "an isolated requirement library and a Markdown document titled 登录"
            | "隔离需求库和标题为登录的 Markdown 文档" => {
                fs::write(root.join("input.md"), "# 登录\n\nUnicode body 中文🙂\n")?;
            }
            "an isolated requirement library" | "隔离需求库" => {}
            "an isolated requirement library with existing documents"
            | "包含已有文档的隔离需求库" => {
                ensure!(create("Existing")?.status.success());
                before = snapshot(&store)?;
            }
            "an isolated requirement library containing a deleted reserved ID"
            | "包含已删除保留 ID 的隔离需求库" => {
                let first = create("Reserved")?;
                let value = success(&first)?;
                reserved = value["id"].as_str().context("created ID")?.into();
                teshi_engine::delete_requirement_document(&store, &reserved, None)?;
            }
            "an isolated requirement library whose index cannot be replaced"
            | "索引无法替换的隔离需求库" => {
                ensure!(create("Existing")?.status.success());
                // Block the atomic writer's temp path after a valid index exists.
                fs::create_dir(store.join("_teshi.tmp"))?;
                before = snapshot(&store)?;
            }
            "I create a requirement from the document with title 登录, priority P1 and an explicit project, requesting JSON"
            | "我从该文档创建标题为登录且优先级为 P1 的需求并显式指定项目和请求 JSON" =>
            {
                outputs.push(
                    command()
                        .args(["create", "--title", "登录", "--priority", "P1", "--file"])
                        .arg(root.join("input.md"))
                        .arg("--project")
                        .arg(&project)
                        .arg("--json")
                        .output()?,
                );
            }
            "I create a requirement from Markdown on stdin requesting JSON without a project"
            | "我从标准输入的 Markdown 创建需求并请求 JSON 且不指定项目" => {
                let mut child = command()
                    .args(["create", "--title", "Stdin", "--stdin", "--json"])
                    .stdin(Stdio::piped())
                    .spawn()?;
                child
                    .stdin
                    .take()
                    .context("stdin")?
                    .write_all(b"# Stdin\n\nComplete body\n")?;
                outputs.push(child.wait_with_output()?);
            }
            "I create a requirement whose supplied title differs from its Markdown heading requesting JSON"
            | "我创建标题与 Markdown 一级标题不同的需求并请求 JSON" => {
                fs::write(root.join("input.md"), "# Actual\n")?;
                outputs.push(
                    command()
                        .args(["create", "--title", "Mismatch", "--file"])
                        .arg(root.join("input.md"))
                        .arg("--json")
                        .output()?,
                );
            }
            "two clients concurrently create independent requirements requesting JSON"
            | "两个客户端并发创建独立需求并请求 JSON" => {
                let first = command()
                    .args(["create", "--title", "First", "--json"])
                    .spawn()?;
                let second = command()
                    .args(["create", "--title", "Second", "--json"])
                    .spawn()?;
                outputs.push(first.wait_with_output()?);
                outputs.push(second.wait_with_output()?);
            }
            "I create a requirement requesting JSON" | "我创建需求并请求 JSON" => {
                outputs.push(create("Cannot persist")?)
            }
            "creation succeeds and JSON contains its stable ID, path, revision and planned project association"
            | "创建成功且 JSON 包含稳定 ID、路径、版本和 planned 项目关联" => {
                let value = success(outputs.last().context("output")?)?;
                ensure!(
                    value["id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("doc-"))
                );
                ensure!(
                    value["revision"]
                        .as_str()
                        .is_some_and(|rev| !rev.is_empty())
                );
                ensure!(value["priority"] == "P1");
                let projects = value["projects"].as_object().context("projects")?;
                ensure!(projects.len() == 1);
                ensure!(projects.values().next().unwrap() == "planned");
                ensure!(
                    fs::read_to_string(store.join(value["path"].as_str().context("path")?))?
                        == fs::read_to_string(root.join("input.md"))?
                );
            }
            "creation succeeds and the requirement has no project association"
            | "创建成功且需求没有项目关联" => {
                let value = success(outputs.last().context("output")?)?;
                ensure!(
                    value["projects"]
                        .as_object()
                        .context("projects")?
                        .is_empty()
                );
                ensure!(
                    fs::read_to_string(store.join(value["path"].as_str().context("path")?))?
                        == "# Stdin\n\nComplete body\n"
                );
            }
            "creation fails with a JSON diagnostic and existing documents remain unchanged"
            | "创建失败并返回 JSON 诊断且已有文档保持不变"
            | "creation fails and no new Markdown document remains"
            | "创建失败且不残留新增 Markdown 文档" => {
                let output = outputs.last().context("output")?;
                ensure!(!output.status.success());
                let diagnostic: Value = serde_json::from_slice(&output.stdout)?;
                ensure!(
                    diagnostic["code"]
                        .as_str()
                        .is_some_and(|code| !code.is_empty())
                );
                ensure!(
                    diagnostic["error"]
                        .as_str()
                        .is_some_and(|error| !error.is_empty())
                );
                ensure!(
                    snapshot(&store)? == before,
                    "store changed after rejected creation"
                );
            }
            "both succeed with different IDs and paths and neither reuses the reserved ID"
            | "两次均成功且 ID 和路径不同并且不复用保留 ID" => {
                ensure!(outputs.len() == 2);
                let first = success(&outputs[0])?;
                let second = success(&outputs[1])?;
                ensure!(first["id"] != second["id"] && first["path"] != second["path"]);
                for value in [&first, &second] {
                    ensure!(value["id"] != reserved);
                    let title = if value == &first { "First" } else { "Second" };
                    ensure!(
                        fs::read_to_string(store.join(value["path"].as_str().context("path")?))?
                            == format!("# {title}\n\n")
                    );
                }
                let index: Value = serde_json::from_slice(&fs::read(store.join("_teshi.json"))?)?;
                ensure!(
                    index["documents"]
                        .as_array()
                        .context("index documents")?
                        .len()
                        == 2
                );
            }
            _ => bail!(
                "unsupported creation step at line {}: {text}",
                step.line_number
            ),
        }
    }
    Ok(())
}

fn success(output: &Output) -> Result<Value> {
    ensure!(
        output.status.success(),
        "CLI failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout)?;
    ensure!(value["ok"] == true);
    Ok(value)
}

fn snapshot(store: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(store)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_file() && (name.ends_with(".md") || name.ends_with(".json")) {
            files.push((name, fs::read(entry.path())?));
        }
    }
    files.sort();
    Ok(files)
}
