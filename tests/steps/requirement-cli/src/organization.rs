//! Process-backed organization steps shared by both Feature locales.
use crate::world::World;
use anyhow::{Result, ensure};

pub(crate) fn handle(world: &mut World, text: &str) -> Result<bool> {
    match text {
        "the tester creates folder AI/Chat" | "测试人员创建功能目录 AI/Chat" => {
            world.run(&["organize", "--folder", "AI/Chat"])?;
        }
        "the tester assigns the login requirement to AI/Chat"
        | "测试人员将登录需求关联到 AI/Chat" => {
            world.run(&["organize", "--assign", "doc-12", "--to", "AI/Chat"])?;
        }
        "the tester creates a planned login query collection"
        | "测试人员创建计划中登录需求的条件集合" => {
            world.run(&[
                "organize",
                "--query",
                "Pending login",
                "--condition",
                "status=planned & title=Login",
            ])?;
        }
        "the tester creates a query with an invalid status"
        | "测试人员创建包含无效状态的条件集合" => {
            world.run(&[
                "organize",
                "--query",
                "Invalid",
                "--condition",
                "status=unknown",
            ])?;
        }
        "the tester reads project B organization" | "测试人员读取项目 B 的需求分类" => {
            let root = world.project_path("B")?;
            teshi_engine::ensure_requirement_project(&root)?;
            world.run(&["organize", "--project", root.to_str().unwrap()])?;
        }
        "the folder hierarchy contains one login reference"
        | "功能目录层级包含一个登录需求引用" => {
            let value: serde_json::Value = serde_json::from_str(&world.last()?.stdout)?;
            let collections = value["collections"].as_array().unwrap();
            ensure!(collections.len() == 2);
            let parent = collections.iter().find(|c| c["name"] == "AI").unwrap();
            let child = collections.iter().find(|c| c["name"] == "Chat").unwrap();
            ensure!(child["parent"] == parent["id"]);
            ensure!(child["documents"] == serde_json::json!(["doc-12"]));
        }
        "the query retains both conditions" | "条件集合保留两个查询条件" => {
            let value: serde_json::Value = serde_json::from_str(&world.last()?.stdout)?;
            let query = &value["collections"][0];
            ensure!(query["condition"]["all"].as_array().unwrap().len() == 2);
            ensure!(query["documents"].as_array().unwrap().is_empty());
        }
        "project B organization is empty" | "项目 B 的需求分类为空" => {
            let value: serde_json::Value = serde_json::from_str(&world.last()?.stdout)?;
            ensure!(value["collections"].as_array().unwrap().is_empty());
        }
        _ => return Ok(false),
    }
    Ok(true)
}
