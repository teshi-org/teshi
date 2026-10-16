//! Outline rows execute independently in real terminals and retain source evidence.
use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use teshi_tui_runner::scenario::{Case, Request, run};

#[test]
fn outline_child() {
    if std::env::var_os("TESHI_OUTLINE_FIXTURE").is_none() {
        return;
    }
    let mut stdout = std::io::stdout();
    writeln!(stdout, "\x1b[2J\x1b[Hfresh-outline-terminal").unwrap();
    stdout.flush().unwrap();
    for line in std::io::stdin().lock().lines() {
        writeln!(stdout, "answer-{}", line.unwrap()).unwrap();
        stdout.flush().unwrap();
    }
}

#[test]
fn english_and_chinese_rows_continue_after_failure_and_filter_by_source_line() {
    for chinese in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let source = if chinese {
            "# language: zh-CN\n功能: 参数化终端\n  背景:\n    假如 新终端\n  场景大纲: 回复\n    当 输入<value>\n    那么 显示<expected>\n    例子:\n      | value | expected |\n      | one | answer-one |\n      | two | missing-answer |\n    例子:\n      | value | expected |\n      | three | answer-three |\n"
        } else {
            "# language: en\nFeature: Parameterized terminal\n  Background:\n    Given fresh terminal\n  Scenario Outline: Reply\n    When send <value>\n    Then see <expected>\n    Examples:\n      | value | expected |\n      | one | answer-one |\n      | two | missing-answer |\n    Examples:\n      | value | expected |\n      | three | answer-three |\n"
        };
        std::fs::write(directory.path().join("outline.feature"), source).unwrap();
        let (ready, send, see, name) = if chinese {
            ("新终端", "输入<value>", "显示<expected>", "回复")
        } else {
            ("fresh terminal", "send <value>", "see <expected>", "Reply")
        };
        let bindings = json!({"version":1,"mode":"tui","steps":[
            {"line":4,"text":ready,"confirmed":true,"actions":[
                {"action":"launch","config":{"program":std::env::current_exe().unwrap(),"args":["--exact","outline_child","--nocapture"],"env":[["TESHI_OUTLINE_FIXTURE","1"]],"cols":80,"rows":24}},
                {"action":"expect_text","text":"fresh-outline-terminal","visible":true,"timeout_ms":5000},
                {"action":"expect_text","text":"answer-","visible":false,"timeout_ms":50}
            ]},
            {"line":6,"text":send,"confirmed":true,"actions":[{"action":"write","text":"<value>"},{"action":"key","key":"Enter"}]},
            {"line":7,"text":see,"confirmed":true,"actions":[{"action":"expect_text","text":"<expected>","visible":true,"timeout_ms":200}]}
        ]});
        std::fs::write(
            directory.path().join("outline.tui.json"),
            serde_json::to_vec(&bindings).unwrap(),
        )
        .unwrap();
        let request = |line_number| Request {
            command: "run".into(),
            cases: vec![Case {
                id: "outline".into(),
                feature_path: "outline.feature".into(),
                scenario: name.into(),
                line_number,
                until_line: None,
            }],
            meta: HashMap::from([(
                "project_root".into(),
                directory.path().to_string_lossy().into_owned(),
            )]),
        };
        let mut output = Vec::new();
        assert_eq!(run(request(None), &mut output).unwrap(), 1);
        let events: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let rows: Vec<_> = events
            .iter()
            .filter(|e| e["type"] == "end_example")
            .collect();
        assert_eq!(
            rows.iter()
                .map(|e| e["status"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["passed", "failed", "passed"]
        );
        assert_eq!(rows[1]["example_line"], 11);
        let failure = events.iter().find(|e| e["type"] == "case_failed").unwrap();
        let path = failure["error"]["attachments"][0]["path"].as_str().unwrap();
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .contains("answer-two")
        );
        assert_eq!(events.last().unwrap()["failed"], 1);
        let mut output = Vec::new();
        assert_eq!(run(request(Some(14)), &mut output).unwrap(), 0);
        let events: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            events.iter().filter(|e| e["type"] == "end_example").count(),
            1
        );
        assert!(events.iter().any(|e| e["type"] == "start_step"
            && e["text"] == if chinese { "输入three" } else { "send three" }));
    }
}
