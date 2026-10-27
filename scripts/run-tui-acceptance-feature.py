#!/usr/bin/env python3
"""Run source-pinned Feature scenarios through dedicated real PTY tests.

Only the scenarios whose assertions cover the complete Feature are mapped.
Unknown or changed scenarios fail closed. Each configured test binary must be
an explicitly built acceptance test executable.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

PINNED = {
    "external_feature_sync.feature": "7601ec883d40fcacf84965ac4ce19e9849148bb951dd8451ab155f2f115bd1ef",
    "外部功能文件同步.feature": "3a881ddd6a9fc202c61d80769eede754ec7d50ea2fc70e6a0c9d1c8677b30fda",
    "footer_actions.feature": "1f0100f227db5be0e8b292c36e3500a25ac97525a0da96449637b88ab80fde31",
    "底部操作.feature": "b4c69512255c94c5102e0a140a32b5c4eec9cbfc67404afdd2ccd8f32184b0fc",
    "chat_profile_title.feature": "093d6f8271f782263365a171360d6ca221e228378c669ef3373c1e71545eadeb",
    "聊天配置标题.feature": "461ad953009d281c0191b2c02954c9952b1e6df2a7f59de0949a8d3c1b6c4482",
    "chat_focus_hint.feature": "f1f3ab75f1bef7ab39a43e6c089623e627a314428dc849aa1cd2e457af2d5c91",
    "聊天聚焦提示.feature": "c4512d520d12ace4d1dc58a7bf3b5bb67181847fda4647e690ee5170cc2ea3ff",
    "chat_runtime_context.feature": "853785c76c4d5725817badaa2021a2281f174ad0218891ac733b35da296cdc15",
    "聊天耗时与上下文.feature": "6b37be89d1bf9c487cbca7f1987d7eb7785e6471b9c5d207ad7046888271dd67",
    "explore_horizontal_navigation.feature": "ee8a67ccd32e1f04370b5b363d99a81ee1ee91186a9fbee94a2bef6a6621af58",
    "步骤横向导航.feature": "f1bb269cebe26b5827a799301895829a91316ddfbb14abf8c276db94a72b32b1",
    "combined_detail_layout.feature": "7541561a9e998460a9e9c20b63fd2ad06ee27d0de18c79a91e823fe2639137c6",
    "合并细节布局.feature": "4e30c221daf2a69178ef0ba883ea1f459199c6c469be710907ebb119b378999e",
    "compact_terminal_layout.feature": "cb0fae8cea9f23f41d5416601ab5d74a1f189646a1eabbf20b5837d8493ca985",
    "小屏终端布局.feature": "50a28a98e2313b99fbb3079ffc6fe4743c092e15c162d320f69b3f40f4aee61e",
    "pointer_feedback.feature": "4a93d91ea450178337701142c717cf64332943a64a80dcece1a921e7dcc59971",
    "点击悬停反馈.feature": "44f8bc65fac9f3a5954fae62c7b526576e6f3423a57e98c16f469ead4a811cda",
    "chat_acp_status_row.feature": "7332a4c378ca9b26154395ae6fc10617af4062cf2bdc35f0ab3c6a107ff27923",
    "聊天ACP状态行.feature": "0ce4738639a7b2b162613cbbc16b75deb81210c21c5393be5ab923d5ed65564e",
}
SCENARIOS = {
    "Discover additions in the configured scope": "external_feature_additions_are_discovered_in_recursive_project",
    "发现配置范围内新增文件": "external_feature_additions_are_discovered_in_recursive_project",
    "Keep a dirty deleted buffer": "externally_deleted_dirty_feature_can_keep_local_edits",
    "保留已删除文件的脏缓冲区": "externally_deleted_dirty_feature_can_keep_local_edits",
    "Defer changes during a run": "external_changes_wait_until_real_feature_run_finishes",
    "运行期间延后同步": "external_changes_wait_until_real_feature_run_finishes",
    "Click a visible action": "requirements_footer_save_and_keyboard_both_persist_documents",
    "点击可见操作": "requirements_footer_save_and_keyboard_both_persist_documents",
    "Clipped actions are not clickable": "clipped_footer_blank_space_does_not_trigger_hidden_action",
    "截断操作不可点击": "clipped_footer_blank_space_does_not_trigger_hidden_action",
    "default title": "builtin_default_title_has_no_profile_suffix",
    "默认标题": "builtin_default_title_has_no_profile_suffix",
    "custom profile title": "changing_custom_profile_updates_title",
    "自定义配置标题": "changing_custom_profile_updates_title",
    "A custom profile named Default stays identifiable": "custom_default_profile_is_identified_in_title",
    "名为Default的自定义配置仍可识别": "custom_default_profile_is_identified_in_title",
    "Focus does not add border hints": "repeated_focus_keeps_input_title_and_context_statistics",
    "聚焦不增加边框提示": "repeated_focus_keeps_input_title_and_context_statistics",
    "Retain send and newline controls": "newline_message_is_sent_once_with_exact_body",
    "保留发送和换行": "newline_message_is_sent_once_with_exact_body",
    "Unknown usage is not zero": "acp_without_reported_usage_shows_unknown_after_response",
    "未知用量不冒充零": "acp_without_reported_usage_shows_unknown_after_response",
    "Show task elapsed time across tool work": "elapsed_time_advances_through_acp_tool_activity_and_remains_after_completion",
    "展示包含工具过程的任务耗时": "elapsed_time_advances_through_acp_tool_activity_and_remains_after_completion",
    "Show reported occupancy and threshold": "reported_context_usage_above_threshold_is_visible",
    "展示实际占用与阈值": "reported_context_usage_above_threshold_is_visible",
    "Clamp at the content boundary": "explore_long_step_horizontal_scroll_and_resize_stays_bounded",
    "停在内容边界": "explore_long_step_horizontal_scroll_and_resize_stays_bounded",
    "Reach long Step text": "explore_long_step_horizontal_scroll_and_resize_stays_bounded",
    "访问长步骤文本": "explore_long_step_horizontal_scroll_and_resize_stays_bounded",
    "Combine supporting details in two columns": "normal_tabs_place_navigation_left_and_details_vertically_right",
    "两列组合辅助细节": "normal_tabs_place_navigation_left_and_details_vertically_right",
    "Keep all panes reachable on a small screen": "narrow_requirements_panes_remain_reachable_with_selection_and_edit",
    "小屏所有面板仍可访问": "narrow_requirements_panes_remain_reachable_with_selection_and_edit",
    "Keep focused content accessible": "narrow_requirements_panes_remain_reachable_with_selection_and_edit",
    "保证聚焦内容可访问": "narrow_requirements_panes_remain_reachable_with_selection_and_edit",
    "Highlight a clickable target": "pointer_hover_changes_visible_action_style_and_clears_on_exit",
    "高亮可点击目标": "pointer_hover_changes_visible_action_style_and_clears_on_exit",
    "shared activity row": "elapsed_time_advances_through_acp_tool_activity_and_remains_after_completion",
    "同行活动状态": "elapsed_time_advances_through_acp_tool_activity_and_remains_after_completion",
}


def emit(event):
    print(json.dumps(event, ensure_ascii=False), flush=True)


def main():
    request = json.loads(sys.stdin.readline())
    cases = request["cases"]
    root = Path(request["meta"]["project_root"])
    emit({"type": "start_run", "total": len(cases)})
    passed = failed = 0
    for case in cases:
        case_id, name = case["id"], case["scenario"]
        emit({"type": "start_case", "case_id": case_id, "name": name})
        start = time.monotonic()
        try:
            feature = root / case["feature_path"]
            if feature.name in {"footer_actions.feature", "底部操作.feature"}:
                env_name = "TESHI_FOOTER_TEST_BIN"
            elif feature.name in {"chat_profile_title.feature", "聊天配置标题.feature"}:
                env_name = "TESHI_CHAT_TITLE_TEST_BIN"
            elif name in {"Show task elapsed time across tool work", "展示包含工具过程的任务耗时", "Show reported occupancy and threshold", "展示实际占用与阈值", "shared activity row", "同行活动状态"}:
                env_name = "TESHI_CHAT_RUNTIME_TEST_BIN"
            elif feature.name in {"chat_focus_hint.feature", "聊天聚焦提示.feature", "chat_runtime_context.feature", "聊天耗时与上下文.feature"}:
                env_name = "TESHI_CHAT_FOCUS_TEST_BIN"
            else:
                env_name = "TESHI_EXTERNAL_SYNC_TEST_BIN"
            executable = os.environ.get(env_name)
            expected = PINNED.get(feature.name)
            if expected is None or hashlib.sha256(feature.read_bytes()).hexdigest() != expected:
                raise ValueError("Feature source changed or is not bound; review the binding")
            test = SCENARIOS.get(name)
            if test is None:
                raise ValueError("scenario is not bound to an acceptance test")
            if not executable or not Path(executable).is_file():
                raise ValueError(f"{env_name} must point to the built PTY test executable")
            child_env = os.environ.copy()
            artifact_root = os.environ.get("TESHI_FEATURE_ARTIFACTS")
            artifact_dir = None
            if artifact_root:
                artifact_dir = Path(artifact_root) / re.sub(r"[^A-Za-z0-9_-]", "_", case_id)
                artifact_dir.mkdir(parents=True, exist_ok=True)
                for key in ("TESHI_P4_P5_ARTIFACTS", "TESHI_REQUIREMENT_CREATION_ARTIFACTS",
                            "TESHI_CHAT_TITLE_ARTIFACTS", "TESHI_CHAT_FOCUS_ARTIFACTS", "TESHI_CHAT_RUNTIME_ARTIFACTS"):
                    child_env[key] = str(artifact_dir)
                (artifact_dir / "test-identity.json").write_text(json.dumps({
                    "test_binary": executable,
                    "test_binary_sha256": hashlib.sha256(Path(executable).read_bytes()).hexdigest(),
                    "teshi_binary": child_env.get("TESHI_SELF_TEST_BIN"),
                    "teshi_binary_sha256": hashlib.sha256(Path(child_env["TESHI_SELF_TEST_BIN"]).read_bytes()).hexdigest()
                    if child_env.get("TESHI_SELF_TEST_BIN") else None,
                    "feature_sha256": expected,
                    "test_name": test,
                }, indent=2) + "\n", encoding="utf-8")
            result = subprocess.run([executable, test, "--exact", "--ignored", "--nocapture"],
                                    capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120,
                                    env=child_env)
            if artifact_dir:
                (artifact_dir / "test.log").write_text(result.stdout + "\n" + result.stderr, encoding="utf-8")
            if result.returncode or "1 passed; 0 failed" not in result.stdout:
                raise RuntimeError(f"PTY test failed or did not run: {result.stdout}\n{result.stderr}")
            passed += 1
            emit({"type": "case_passed", "case_id": case_id, "duration_ms": int((time.monotonic()-start)*1000)})
        except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
            failed += 1
            emit({"type": "case_failed", "case_id": case_id, "duration_ms": int((time.monotonic()-start)*1000),
                  "error": {"message": str(error), "attachments": []}})
    emit({"type": "end_run", "passed": passed, "failed": failed, "skipped": 0})
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
