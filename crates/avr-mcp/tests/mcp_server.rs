//! 进程内 MCP 集成测试：rmcp 0.3 客户端经 tokio duplex 连接 `AvrMcpServer`，
//! 走一遍 analyze_media / read_transcript / get_frame / resources/read，
//! 以及路径穿越用例（list_visual_hints / extract_frames / read_transcript / get_frame / resource）。

mod common;

use avr_mcp::server::AvrMcpServer;
use rmcp::model::{
    CallToolRequestParam, CallToolResult, ReadResourceRequestParam, ResourceContents,
};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};

type Client = RunningService<RoleClient, ()>;

async fn connect(server: AvrMcpServer) -> Client {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        let running = server.serve(server_io).await.expect("server start");
        let _ = running.waiting().await;
    });
    ().serve(client_io).await.expect("client connect")
}

async fn call(client: &Client, name: &'static str, args: Value) -> CallToolResult {
    client
        .call_tool(CallToolRequestParam {
            name: name.into(),
            arguments: args.as_object().cloned(),
        })
        .await
        .expect("call_tool transport")
}

/// 第一个 text content 解析成 JSON。
fn json_of(r: &CallToolResult) -> Value {
    let text = r.content[0].as_text().expect("text content").text.clone();
    serde_json::from_str(&text).expect("json")
}

fn is_error(v: &Value) -> bool {
    v.get("error") == Some(&Value::Bool(true))
}

#[tokio::test]
async fn mcp_tools_end_to_end_and_traversal_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = common::test_ctx(tmp.path());
    // 有 ffmpeg 时用真实视频并让 get_frame / resource 真正截帧；否则只测文本链路。
    let ff = common::ffmpeg_or_skip();
    let media = match &ff {
        Some(ff) => {
            ctx.config.tools.ffmpeg_path = ff.ffmpeg.display().to_string();
            ctx.config.tools.ffprobe_path = ff.ffprobe.display().to_string();
            common::make_video(ff, tmp.path())
        }
        None => {
            let p = tmp.path().join("clip.mp4");
            std::fs::write(&p, b"fake").unwrap();
            p
        }
    };
    let client = connect(AvrMcpServer::new(ctx, common::fake_deps())).await;

    // tools/list
    let tools = client.list_all_tools().await.unwrap();
    let names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    for n in [
        "analyze_media",
        "read_transcript",
        "get_frame",
        "list_visual_hints",
        "extract_frames",
    ] {
        assert!(names.contains(&n.to_string()), "缺少工具 {n}: {names:?}");
    }

    // analyze_media 默认 compact。
    let src = media.display().to_string();
    let compact = json_of(&call(&client, "analyze_media", json!({ "source": src })).await);
    assert!(!is_error(&compact), "{compact}");
    assert!(compact.get("text").is_some() && compact.get("sample_segments").is_some());
    assert_eq!(compact["response_mode"], "compact");

    // 显式 auto：基线转写很短（远低于默认 2000），直接返回 full。
    let auto = json_of(
        &call(
            &client,
            "analyze_media",
            json!({ "source": src, "response_mode": "auto" }),
        )
        .await,
    );
    assert_eq!(auto["response_mode"], "full", "{auto}");

    // 显式 index。
    let index = json_of(
        &call(
            &client,
            "analyze_media",
            json!({ "source": src, "response_mode": "index" }),
        )
        .await,
    );
    assert!(index.get("sections").is_some() && index.get("text").is_none());
    let key = index["media_key"].as_str().unwrap().to_string();
    assert_eq!(key, compact["media_key"]);

    // read_transcript 分页取回全部分段。
    let mut total = 0;
    let mut pages = 0;
    let mut cursor: Option<String> = None;
    loop {
        let mut args = json!({ "media_key": key, "max_chars": 300 });
        if let Some(c) = &cursor {
            args["cursor"] = json!(c);
        }
        let page = json_of(&call(&client, "read_transcript", args).await);
        pages += 1;
        assert!(!is_error(&page), "{page}");
        total += page["segments"].as_array().unwrap().len();
        match page.get("nextCursor").and_then(Value::as_str) {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
    }
    assert_eq!(total, index["segment_count"].as_u64().unwrap() as usize);

    // get_frame + resources/read（需要真实 ffmpeg）。
    if ff.is_some() {
        let r = call(
            &client,
            "get_frame",
            json!({ "media_key": key, "n": 0, "width": 160 }),
        )
        .await;
        let info = json_of(&r);
        assert!(!is_error(&info), "{info}");
        assert_eq!(info["resource_uri"], index["frames"][0]["resource_uri"]);
        assert!(
            r.content.iter().any(|c| c.as_image().is_some()),
            "应带 JPEG 图片"
        );

        let uri = info["resource_uri"].as_str().unwrap().to_string();
        let res = client
            .read_resource(ReadResourceRequestParam { uri: uri.clone() })
            .await
            .unwrap();
        match &res.contents[0] {
            ResourceContents::BlobResourceContents {
                uri: u,
                mime_type,
                blob,
            } => {
                assert_eq!(u, &uri);
                assert_eq!(mime_type.as_deref(), Some("image/jpeg"));
                assert!(!blob.is_empty());
            }
            other => panic!("应为 blob: {other:?}"),
        }
        println!("mcp e2e: frames RAN");
    }

    // 路径穿越：所有接收 media_key 的工具都必须拒绝。
    for bad in [
        "..",
        "../..",
        "../etc",
        "a/../../b",
        "/etc",
        ".hidden",
        "..\\x",
    ] {
        for (tool, args) in [
            ("list_visual_hints", json!({ "media_key": bad })),
            (
                "extract_frames",
                json!({ "media_key": bad, "times": [0.0] }),
            ),
            ("read_transcript", json!({ "media_key": bad })),
            ("get_frame", json!({ "media_key": bad, "n": 0 })),
        ] {
            let v = json_of(&call(&client, tool, args).await);
            assert!(is_error(&v), "{tool}({bad:?}) 应被拒绝: {v}");
        }
        let res = client
            .read_resource(ReadResourceRequestParam {
                uri: format!("avr://job/{bad}/frame/0"),
            })
            .await;
        assert!(res.is_err(), "resource {bad:?} 应被拒绝");
    }

    client.cancel().await.unwrap();

    // 调用日志：每次 tools/call 一行 JSON（resources/read 不记）。
    let log_path = tmp.path().join("root").join("logs").join("mcp-calls.jsonl");
    let lines: Vec<Value> = std::fs::read_to_string(&log_path)
        .expect("调用日志文件")
        .lines()
        .map(|l| serde_json::from_str(l).expect("每行是 JSON"))
        .collect();
    let calls = 3 + pages + usize::from(ff.is_some()) + 7 * 4;
    assert_eq!(lines.len(), calls, "日志行数应等于工具调用次数");
    for l in &lines {
        for k in ["ts", "tool", "args", "bytes", "ok"] {
            assert!(l.get(k).is_some(), "缺字段 {k}: {l}");
        }
    }
    let modes: Vec<_> = lines
        .iter()
        .filter(|l| l["tool"] == "analyze_media")
        .map(|l| l["response_mode"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(modes, ["compact", "full", "index"]);
    assert!(lines
        .iter()
        .all(|l| l["tool"] != "analyze_media" || l["media_key"] == json!(key)));
    let rt = lines
        .iter()
        .find(|l| l["tool"] == "read_transcript")
        .unwrap();
    assert_eq!(rt["media_key"], json!(key));
    assert_eq!(rt["args"]["max_chars"], 300);
    assert!(rt["bytes"].as_u64().unwrap() > 0 && rt["ok"] == true);
    assert!(lines
        .iter()
        .any(|l| l["ok"] == false && l["media_key"] == ".."));
}
