//! Export confirmed step-bindings to external test projects (behave + UIA).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::{ExportArgs, ExportTargetArg};
use anyhow::{Context, Result, bail};
use teshi_runtime::{StepBinding, list_step_bindings, normalize_step_text, resolve_step_bindings};

/// Handles `teshi export ...` subcommands.
pub fn handle_export_command(args: &ExportArgs) -> Result<()> {
    let project_root = std::env::current_dir().context("resolve current directory")?;
    match args.target {
        ExportTargetArg::Behave => export_behave(&project_root, args),
    }
}

fn export_behave(project_root: &Path, args: &ExportArgs) -> Result<()> {
    let feature_rel = args.feature.replace('\\', "/");
    let feature_abs = project_root.join(&feature_rel);
    if !feature_abs.is_file() {
        bail!("feature file not found: {}", feature_abs.display());
    }
    let bindings = list_step_bindings(project_root, &feature_rel)?;
    let confirmed = resolve_step_bindings(project_root, &feature_rel, None)?;
    if confirmed.is_empty() {
        bail!("no confirmed bindings for {feature_rel}; record locators in teshi first");
    }

    let out = PathBuf::from(&args.out);
    let features_dir = out.join("features");
    let feature_steps_dir = features_dir.join("steps");
    let pages_dir = out.join("pages");
    fs::create_dir_all(&features_dir)?;
    fs::create_dir_all(&feature_steps_dir)?;
    fs::create_dir_all(&pages_dir)?;

    let feature_name = feature_abs
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("feature.feature");
    fs::copy(&feature_abs, features_dir.join(feature_name)).context("copy feature file")?;

    let page_module = page_module_name(&stem(feature_name));
    let page_class = pascal_case(&page_module) + "Page";

    if args.with_po {
        let page_py = render_page_py(&page_class, &confirmed);
        fs::write(pages_dir.join(format!("{page_module}_page.py")), page_py)?;
        fs::write(pages_dir.join("__init__.py"), "")?;
    }

    let steps_py = render_steps_py(&page_module, &page_class, args.with_po, &confirmed)?;
    fs::write(
        feature_steps_dir.join(format!("{page_module}_steps.py")),
        steps_py,
    )?;
    fs::write(feature_steps_dir.join("__init__.py"), "")?;

    fs::write(features_dir.join("environment.py"), ENVIRONMENT_PY)?;
    fs::write(out.join("behave.ini"), BEHAVE_INI)?;
    fs::write(out.join("requirements.txt"), REQUIREMENTS_TXT)?;
    fs::write(out.join(".env.example"), ENV_EXAMPLE)?;
    fs::write(
        out.join("README.md"),
        render_readme(&feature_rel, feature_name),
    )?;

    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "target": "behave",
            "out": out.display().to_string(),
            "feature": feature_rel,
            "bindings": bindings.steps.len(),
            "confirmed_steps": confirmed.len()
        })
    );
    Ok(())
}

fn stem(filename: &str) -> String {
    filename
        .strip_suffix(".feature")
        .unwrap_or(filename)
        .to_string()
}

fn snake_case(input: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && prev_lower {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        } else if !ch.is_whitespace() && ch != '_' && ch != '-' {
            if !out.is_empty() && !out.ends_with('_') {
                out.push('_');
            }
            use std::fmt::Write as _;
            let _ = write!(out, "u{:04x}", ch as u32);
            prev_lower = false;
        } else {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    out.trim_matches('_').to_string()
}

/// Stable non-empty module name for page objects (handles non-ASCII feature stems).
fn page_module_name(stem: &str) -> String {
    let snake = snake_case(stem);
    if !snake.is_empty() {
        return snake;
    }
    let hash = stem.bytes().fold(0u64, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(u64::from(b))
    });
    format!("feature_{hash:08x}")
}

fn pascal_case(snake: &str) -> String {
    snake
        .split('_')
        .filter(|s| !s.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(f) => {
                    let mut s = f.to_ascii_uppercase().to_string();
                    s.push_str(chars.as_str());
                    s
                }
            }
        })
        .collect()
}

fn python_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

fn locator_const_name(selector: &str) -> String {
    if let Some(id) = selector.strip_prefix("uia:automation_id=") {
        return snake_case(id);
    }
    let digest = selector
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(24)
        .collect::<String>();
    format!("loc_{digest}")
}

fn render_page_py(page_class: &str, bindings: &[StepBinding]) -> String {
    let mut constants = BTreeMap::new();
    for b in bindings {
        let name = locator_const_name(&b.primary.value);
        constants
            .entry(name)
            .or_insert_with(|| b.primary.value.clone());
    }
    let mut body = String::from("# Generated by teshi; re-export after rebinding.\n\n\n");
    body.push_str(&format!("class {page_class}:\n"));
    body.push_str("    \"\"\"Page object for exported WinUI bindings.\"\"\"\n\n");
    for (name, value) in constants {
        body.push_str(&format!("    {name} = \"{value}\"\n"));
    }
    body
}

fn behave_decorator(keyword: &str) -> &'static str {
    match keyword.trim().to_ascii_lowercase().as_str() {
        "given" | "假如" | "假设" => "given",
        "when" | "当" => "when",
        "then" | "那么" | "则" => "then",
        _ => "step",
    }
}

fn render_action_body(binding: &StepBinding, page_module: &str, with_po: bool) -> Result<String> {
    let const_name = locator_const_name(&binding.primary.value);
    let target = if with_po {
        format!("context.pages.{page_module}.{const_name}")
    } else {
        format!("\"{}\"", python_escape(&binding.primary.value))
    };
    let uia = "context.uia";
    let action = binding.primary.action.as_str();
    let value_arg = binding.primary.value_arg.as_deref();
    let line = match action {
        "click" => format!("{uia}.click({target})"),
        "fill" => {
            let val = value_arg
                .map(python_env_or_literal)
                .unwrap_or_else(|| "\"\"".to_string());
            format!("{uia}.fill({target}, {val})")
        }
        "assert_visible" => format!("{uia}.assert_visible({target})"),
        "assert_text" => {
            let val = value_arg
                .map(python_env_or_literal)
                .unwrap_or_else(|| "\"\"".to_string());
            format!("{uia}.assert_text({target}, {val})")
        }
        "select" => format!("{uia}.select({target})"),
        "press_key" => {
            let val = value_arg
                .map(python_env_or_literal)
                .unwrap_or_else(|| "\"{ENTER}\"".to_string());
            format!("{uia}.press_key({target}, {val})")
        }
        "navigate" => {
            let url = value_arg.unwrap_or(&binding.primary.value);
            format!("{uia}.navigate({})", python_env_or_literal(url))
        }
        "exec" => {
            let cmd = value_arg.ok_or_else(|| {
                anyhow::anyhow!(
                    "exec binding missing value_arg at line {}",
                    binding.step_line
                )
            })?;
            format!(
                "    import subprocess\n    subprocess.run([\"pwsh\", \"-NoProfile\", \"-Command\", {}], check=True)\n",
                python_env_or_literal(cmd)
            )
        }
        other => bail!("unsupported export action: {other}"),
    };
    if action == "exec" {
        return Ok(line);
    }
    Ok(format!("    {line}\n"))
}

fn python_env_or_literal(value: &str) -> String {
    if value.starts_with("${") && value.ends_with('}') {
        let key = &value[2..value.len() - 1];
        format!("os.environ.get(\"{key}\", \"\")")
    } else {
        format!("'{}'", python_escape(value))
    }
}

fn render_steps_py(
    page_module: &str,
    page_class: &str,
    with_po: bool,
    bindings: &[StepBinding],
) -> Result<String> {
    let mut by_norm: BTreeMap<String, &StepBinding> = BTreeMap::new();
    for b in bindings {
        let key = normalize_step_text(&b.step_text);
        by_norm.entry(key).or_insert(b);
    }

    let mut out = String::from("# Generated by teshi; re-export after rebinding.\n\n");
    out.push_str("import os\n\n");
    out.push_str("from behave import given, step, then, when\n\n");
    if with_po {
        out.push_str(&format!(
            "from pages.{page_module}_page import {page_class}\n\n"
        ));
    }
    out.push_str(&format!(
        "def _ensure_page(context):\n    if not hasattr(context, \"pages\"):\n        context.pages = type(\"Pages\", (), {{}})()\n    if not hasattr(context.pages, \"{page_module}\"):\n        context.pages.{page_module} = {page_class}()\n\n\n",
        page_module = page_module,
        page_class = page_class
    ));

    for b in by_norm.values() {
        let deco = behave_decorator(&b.step_keyword);
        let fn_name = format!("step_{}", snake_case(&normalize_step_text(&b.step_text)));
        let step_literal = python_escape(&b.step_text);
        out.push_str(&format!("@{deco}('{step_literal}')\n"));
        out.push_str(&format!("def {fn_name}(context):\n"));
        if with_po {
            out.push_str("    _ensure_page(context)\n");
        }
        out.push_str(&render_action_body(b, page_module, with_po)?);
        out.push('\n');
    }
    Ok(out)
}

fn render_readme(feature_rel: &str, feature_name: &str) -> String {
    format!(
        "# Exported WinUI behave tests\n\n\
         Generated from teshi `step-bindings` for `{feature_rel}`.\n\n\
         ## Setup\n\n\
         1. Copy `.env.example` to `.env` and set `APP_EXE`.\n\
         2. `python -m venv .venv` and `pip install -r requirements.txt`.\n\n\
         ## Run\n\n\
         ```bash\n\
         cd tests-e2e\n\
         behave\n\
         behave features/{feature_name}\n\
         ```\n\n\
         ## After editing step files\n\n\
         Clear stale bytecode before re-running:\n\n\
         ```powershell\n\
         Get-ChildItem -Recurse __pycache__ | Remove-Item -Recurse -Force\n\
         ```\n\n\
         Re-run `teshi export --target behave` after rebinding in teshi.\n"
    )
}

const BEHAVE_INI: &str = "[behave]\npaths = features\n";

const REQUIREMENTS_TXT: &str = "behave>=1.2.6\nuiautomation>=2.0.18\n";

const ENV_EXAMPLE: &str = "# Path to the WinUI3 app under test\n\
APP_EXE=C:\\\\path\\\\to\\\\YourApp.exe\n\
# Optional secrets referenced as ${VAR} in bindings\n\
# TEST_PASSWORD=secret\n\
LAUNCH_TIMEOUT_MS=15000\n";

const ENVIRONMENT_PY: &str = r#"# Generated by teshi; re-export after rebinding.
import os
import sys
import time
from pathlib import Path

# Allow `from pages...` imports when behave loads steps from features/steps/.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from uiautomation import Control, WindowControl


class UiaDriver:
    """Thin UIA helper aligned with teshi winapp actions."""

    def __init__(self):
        self._root = None

    def launch(self, exe_path: str):
        import subprocess

        subprocess.Popen([exe_path], shell=False)
        timeout_ms = int(os.environ.get("LAUNCH_TIMEOUT_MS", "15000"))
        deadline = time.time() + timeout_ms / 1000.0
        while time.time() < deadline:
            self._root = WindowControl(searchDepth=1, Name=None)
            if self._root.Exists(0, 0):
                return
            time.sleep(0.25)
        raise RuntimeError(f"no window appeared after launching {exe_path}")

    def _parse_selector(self, selector: str) -> dict:
        if not selector.startswith("uia:"):
            raise NotImplementedError(f"unsupported selector in export: {selector}")
        body = selector[4:]
        if "=" not in body:
            raise NotImplementedError(f"unsupported selector in export: {selector}")
        key, value = body.split("=", 1)
        if key == "control_type" and ";" in value:
            parts = {}
            for segment in value.split(";"):
                if "=" in segment:
                    k, v = segment.split("=", 1)
                    parts[k.strip()] = v.strip()
            return parts
        return {key: value}

    def _control(self, selector: str) -> Control:
        props = self._parse_selector(selector)
        if "automation_id" in props:
            return WindowControl(AutomationId=props["automation_id"], searchDepth=32)
        if "name" in props and len(props) == 1:
            return WindowControl(Name=props["name"], searchDepth=32)
        if "control_type" in props:
            kwargs = {"searchDepth": 32}
            if props.get("name"):
                kwargs["Name"] = props["name"]
            return WindowControl(**kwargs)
        if "path" in props:
            raise NotImplementedError(
                f"path selectors are brittle in exported behave tests: {selector}"
            )
        raise NotImplementedError(f"unsupported selector in export: {selector}")

    def click(self, selector: str):
        ctrl = self._control(selector)
        ctrl.Click()

    def fill(self, selector: str, value: str):
        ctrl = self._control(selector)
        ctrl.SetFocus()
        ctrl.SendKeys(value)

    def assert_visible(self, selector: str):
        if not self._control(selector).Exists(0, 0):
            raise AssertionError(f"not visible: {selector}")

    def assert_text(self, selector: str, expected: str):
        ctrl = self._control(selector)
        name = ctrl.Name or ""
        if name != expected:
            raise AssertionError(f"expected Name {expected!r}, got {name!r}")

    def select(self, selector: str):
        self.click(selector)

    def press_key(self, selector: str, keys: str):
        ctrl = self._control(selector)
        ctrl.SetFocus()
        ctrl.SendKeys(keys)

    def navigate(self, url: str):
        raise NotImplementedError("navigate is not supported for WinUI export")


def before_all(context):
    context.uia = UiaDriver()
    exe = os.environ.get("APP_EXE")
    if not exe:
        raise RuntimeError("set APP_EXE in .env before running behave")
    context.uia.launch(exe)


def after_all(context):
    context.uia = None
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_handles_feature_stem() {
        assert_eq!(snake_case("LoginBug"), "login_bug");
    }

    #[test]
    fn snake_case_handles_non_ascii_feature_stem() {
        let name = page_module_name("库界面运行态展示");
        assert!(!name.is_empty());
        assert!(!name.contains(".."));
    }

    #[test]
    fn locator_const_from_automation_id() {
        assert_eq!(
            locator_const_name("uia:automation_id=LoginButton"),
            "login_button"
        );
    }
}
