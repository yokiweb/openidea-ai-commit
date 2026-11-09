//! AI Commit：把 Commit 面板中勾选文件的 diff 交给 DeepSeek，生成规范化的中文
//! 提交信息并写回消息框。只使用官方插件 API 1.3 的 commit 接口，不改 IDE 源码。

pub mod generate;

use generate::Config;
use openidea_plugin::host;
use openidea_plugin::{Error, HttpRequest, Invocation, NotificationLevel, Plugin, register_plugin};

/// 读取声明式设置；schema 默认值由 IDE 提供，这里再兜一层以防缺失。
fn config() -> Config {
    let get = |key: &str, default: &str| {
        host::setting_string(key)
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| default.into())
    };
    let language = get("language", "zh");
    let custom = host::setting_string("systemPrompt").unwrap_or_default();
    let prompt = if custom.trim().is_empty() {
        (if language == "en" { generate::PROMPT_EN } else { generate::PROMPT_ZH }).into()
    } else {
        custom
    };
    let max_diff = host::setting_number("maxDiffChars")
        .map(|n| (n as usize).clamp(1_000, 200_000))
        .unwrap_or(generate::DEFAULT_MAX_DIFF);
    Config {
        endpoint: get("endpoint", generate::DEFAULT_ENDPOINT),
        model: get("model", generate::DEFAULT_MODEL),
        max_diff,
        prompt,
    }
}

#[derive(Default)]
pub struct AiCommit;

impl AiCommit {
    /// 完整链路：勾选 diff → DeepSeek → 写回消息框。
    fn generate(&self) -> Result<String, Error> {
        // 长请求：task 把命令时限从 5s 提到 60s，并让用户可取消。
        let task = host::begin_task("AI Commit", true);
        if !host::secret_is_set("api-key") {
            return Err(Error::Invalid("请先在 设置 › 插件 中填写 DeepSeek API Key".into()));
        }
        let cfg = config();
        let draft =
            host::pending_commit()?.ok_or_else(|| Error::NotFound("当前窗口没有 Git 仓库".into()))?;
        let diff = generate::collect_diff(&draft.changes, cfg.max_diff)?;
        if task.is_cancelled() {
            return Err(Error::Cancelled);
        }
        task.report(0, None, Some("正在生成提交信息"));

        let mut req = HttpRequest::post(&cfg.endpoint, generate::request_body(&cfg, &diff))
            .header("Content-Type", "application/json")
            // host 用系统 keychain 中的真值替换占位符，插件本身读不到 key。
            .header("Authorization", "Bearer {{secret:api-key}}");
        req.timeout_ms = Some(60_000);
        let resp = host::fetch(&req)?;
        if task.is_cancelled() {
            return Err(Error::Cancelled);
        }
        // 不回显响应体：provider 的错误页可能带上代码或 key。
        if resp.status != 200 {
            return Err(Error::Failed(format!("DeepSeek 返回 HTTP {}", resp.status)));
        }
        let message = generate::parse_message(&resp.body)?;
        // revision 保护：期间用户改了草稿或勾选，IDE 返回 Stale，不覆盖用户输入。
        host::set_commit_message(draft.revision, &message)?;
        task.report(1, Some(1), Some("已生成"));
        host::notify(NotificationLevel::Info, "AI Commit", "提交信息已生成，请检查后再提交");
        Ok(message)
    }
}

impl Plugin for AiCommit {
    fn run_command(
        &mut self,
        id: &str,
        _ctx: &Invocation,
        _arg: Option<&str>,
    ) -> Result<Option<String>, Error> {
        match id {
            "generate" => match self.generate() {
                Ok(msg) => Ok(Some(msg)),
                Err(Error::Cancelled) => Ok(None),
                Err(e) => {
                    host::notify(NotificationLevel::Error, "AI Commit", &e.to_string());
                    Err(e)
                }
            },
            _ => Err(Error::NotFound(format!("command {id}"))),
        }
    }
}

register_plugin!(AiCommit);

#[cfg(test)]
mod tests {
    use super::*;
    use openidea_plugin::mock::MockHost;
    use openidea_plugin::{CommitChange, CommitDraft, HttpResponse};
    use serde_json::{Value, json};

    const CAPS: &[&str] = &["vcs.read", "vcs.write", "secrets", "net:api.deepseek.com"];
    const MESSAGE: &str = "feat(task): 新增定时任务\n\n- 增加 ZbjcScheduledTaskMapper.java";

    fn draft(changes: Vec<CommitChange>) -> CommitDraft {
        CommitDraft { revision: 7, message: String::new(), amend: false, changes }
    }

    fn checked_file() -> CommitChange {
        CommitChange {
            repository: "/repo".into(),
            path: "src/a.java".into(),
            old_path: None,
            status: "modified".into(),
            diff: Some("-old\n+new".into()),
            truncated: false,
        }
    }

    /// 装好 mock：有 key、勾选了一个文件、模型返回固定结果。
    fn ready() -> MockHost {
        let host = MockHost::install().with_capabilities(CAPS);
        host.set_secret("api-key", "sk-test");
        host.set_commit_draft(Some(draft(vec![checked_file()])));
        host.on_fetch(|_| Ok(ok_response()));
        host
    }

    fn ok_response() -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![],
            body: json!({"choices": [{"finish_reason": "stop", "message": {"content": MESSAGE}}]})
                .to_string()
                .into_bytes(),
        }
    }

    #[test]
    fn generates_from_checked_files_and_fills_the_message() {
        let host = ready();
        host.on_fetch(|r| {
            assert_eq!(r.url, "https://api.deepseek.com/chat/completions");
            assert_eq!(r.method, "POST");
            assert!(r.headers.contains(&("Authorization".into(), "Bearer sk-test".into())));
            assert_eq!(r.timeout_ms, Some(60_000));
            let body: Value = serde_json::from_slice(r.body.as_ref().unwrap()).unwrap();
            assert_eq!(body["model"], "deepseek-chat");
            assert_eq!(body["stream"], false);
            let diff = body["messages"][1]["content"].as_str().unwrap();
            assert!(diff.contains("src/a.java"));
            assert!(diff.contains("-old\n+new"));
            Ok(ok_response())
        });
        let msg = AiCommit::default()
            .run_command("generate", &host.invocation(), None)
            .unwrap()
            .unwrap();
        assert_eq!(msg, MESSAGE);
        let after = host.commit_draft().unwrap();
        assert_eq!(after.message, MESSAGE, "消息写回输入框");
        assert_eq!(after.revision, 8, "revision 递增");
        assert_eq!(host.requests().len(), 1);
    }

    #[test]
    fn settings_override_endpoint_model_and_language() {
        let host = ready();
        host.set_setting("endpoint", "\"https://api.deepseek.com/v1/chat/completions\"");
        host.set_setting("model", "\"deepseek-reasoner\"");
        host.set_setting("language", "\"en\"");
        host.on_fetch(|r| {
            let body: Value = serde_json::from_slice(r.body.as_ref().unwrap()).unwrap();
            assert_eq!(body["model"], "deepseek-reasoner");
            assert!(body["messages"][0]["content"].as_str().unwrap().starts_with("You generate"));
            Ok(ok_response())
        });
        AiCommit::default().run_command("generate", &host.invocation(), None).unwrap();
        assert_eq!(host.requests()[0].url, "https://api.deepseek.com/v1/chat/completions");
    }

    /// host 的 revision 契约：用户中途改草稿后，旧 revision 的写回被拒绝。
    /// （MockHost 的 fetch handler 持有 RefCell borrow，不能在请求回调里改草稿，
    /// 因此这里直接验证契约本身；插件总是把 pending_commit 的 revision 原样传回。）
    #[test]
    fn host_rejects_stale_revision() {
        let host = ready();
        host.edit_commit_message("用户手写的草稿");
        assert!(matches!(host::set_commit_message(7, MESSAGE), Err(Error::Stale)));
        assert_eq!(host.commit_draft().unwrap().message, "用户手写的草稿");
    }

    #[test]
    fn cancelled_request_does_not_write_the_message() {
        let host = ready();
        host.on_fetch(|_| Err(Error::Cancelled));
        assert_eq!(
            AiCommit::default().run_command("generate", &host.invocation(), None).unwrap(),
            None
        );
        assert_eq!(host.commit_draft().unwrap().message, "");
    }

    #[test]
    fn missing_key_or_repository_stops_before_the_request() {
        let no_key = MockHost::install().with_capabilities(CAPS);
        no_key.set_commit_draft(Some(draft(vec![checked_file()])));
        assert!(AiCommit::default().run_command("generate", &no_key.invocation(), None).is_err());
        assert!(no_key.requests().is_empty(), "没有 key 时不发请求");

        let no_repo = MockHost::install().with_capabilities(CAPS);
        no_repo.set_secret("api-key", "sk-test");
        no_repo.set_commit_draft(None);
        assert!(AiCommit::default().run_command("generate", &no_repo.invocation(), None).is_err());
        assert!(no_repo.requests().is_empty(), "无仓库时不发请求");
    }

    #[test]
    fn no_selection_and_http_errors_keep_the_draft() {
        let empty = MockHost::install().with_capabilities(CAPS);
        empty.set_secret("api-key", "sk-test");
        empty.set_commit_draft(Some(draft(vec![])));
        assert!(AiCommit::default().run_command("generate", &empty.invocation(), None).is_err());
        assert!(empty.requests().is_empty(), "没勾文件不发请求");

        let bad = ready();
        bad.on_fetch(|_| {
            Ok(HttpResponse {
                status: 401,
                headers: vec![],
                body: b"sk-test private-source".to_vec(),
            })
        });
        let err = AiCommit::default().run_command("generate", &bad.invocation(), None).unwrap_err();
        assert_eq!(err.to_string(), "DeepSeek 返回 HTTP 401", "错误不回显响应体");
        assert_eq!(bad.commit_draft().unwrap().message, "", "草稿保持原样");
    }

    #[test]
    fn ungranted_network_is_reported_not_silently_swallowed() {
        let host = MockHost::install().with_capabilities(&["vcs.read", "vcs.write", "secrets"]);
        host.set_secret("api-key", "sk-test");
        host.set_commit_draft(Some(draft(vec![checked_file()])));
        let err = AiCommit::default().run_command("generate", &host.invocation(), None).unwrap_err();
        assert!(matches!(&err, Error::Denied(c) if c == "net:api.deepseek.com"), "{err:?}");
        assert_eq!(host.commit_draft().unwrap().message, "");
    }
}
