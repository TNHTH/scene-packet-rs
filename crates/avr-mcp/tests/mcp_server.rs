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
    ctx.config.mcp.call_log = true; // 文件日志默认关闭，这里显式开启以断言日志行
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

        // P5-2：索引带板书区间；get_frame(board) 一次多张、超过 4 张报错。
        let board = index["board"].as_array().expect("index 应带 board[]");
        assert!(!board.is_empty(), "{index}");
        let b0 = &board[0];
        assert!(b0["end"].as_f64().unwrap() > b0["start"].as_f64().unwrap());
        assert!(b0["transcript"].as_str().unwrap().contains("第1句"));
        let nb = board.len().min(2);
        let ns: Vec<usize> = (0..nb).collect();
        let multi = call(
            &client,
            "get_frame",
            json!({ "media_key": key, "board": true, "ns": ns, "width": 160 }),
        )
        .await;
        let images = multi
            .content
            .iter()
            .filter(|c| c.as_image().is_some())
            .count();
        assert_eq!(images, nb, "{}", json_of(&multi));
        let too_many = json_of(
            &call(
                &client,
                "get_frame",
                json!({ "media_key": key, "board": true, "ns": [0, 1, 2, 3, 4] }),
            )
            .await,
        );
        assert!(is_error(&too_many), "{too_many}");
        let bad = json_of(
            &call(
                &client,
                "get_frame",
                json!({ "media_key": key, "board": true, "n": 999 }),
            )
            .await,
        );
        assert!(is_error(&bad), "{bad}");
        let res = client
            .read_resource(ReadResourceRequestParam {
                uri: b0["resource_uri"].as_str().unwrap().to_string(),
            })
            .await
            .unwrap();
        assert!(matches!(
            &res.contents[0],
            ResourceContents::BlobResourceContents { .. }
        ));

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
    // 有 ffmpeg 时：get_frame 单张 1 次 + 板书多张 / 超限 / 越界 3 次。
    let calls = 3 + pages + 4 * usize::from(ff.is_some()) + 7 * 4;
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
    // 本地路径不以明文出现在日志里（source 只记扩展名）。
    let raw = std::fs::read_to_string(&log_path).unwrap();
    let media_name = media.file_name().unwrap().to_string_lossy().to_string();
    let tmp_dir = tmp.path().display().to_string();
    assert!(
        !raw.contains(&media_name) && !raw.contains(&tmp_dir),
        "日志含明文路径"
    );
    assert!(lines
        .iter()
        .filter(|l| l["tool"] == "analyze_media")
        .all(|l| l["args"]["source"] == "path:.mp4"));
}

#[tokio::test]
async fn call_log_file_off_by_default() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = common::test_ctx(tmp.path());
    let client = connect(AvrMcpServer::new(ctx, common::fake_deps())).await;
    let v = json_of(&call(&client, "read_transcript", json!({ "media_key": "nope" })).await);
    assert!(v.is_object());
    client.cancel().await.unwrap();
    assert!(!tmp
        .path()
        .join("root")
        .join("logs")
        .join("mcp-calls.jsonl")
        .exists());
}

/// P5-2：转写稀少的视频，analyze 首次返回直接附最多 4 张板书图。
#[tokio::test]
async fn sparse_transcript_attaches_board_images() {
    let Some(ff) = common::ffmpeg_or_skip() else {
        return;
    };
    let tmp = tempfile::TempDir::new().unwrap();
    let mut ctx = common::test_ctx(tmp.path());
    ctx.config.tools.ffmpeg_path = ff.ffmpeg.display().to_string();
    ctx.config.tools.ffprobe_path = ff.ffprobe.display().to_string();
    let media = common::make_video(&ff, tmp.path());
    let client = connect(AvrMcpServer::new(ctx, common::sparse_deps())).await;
    let r = call(
        &client,
        "analyze_media",
        json!({ "source": media.display().to_string() }),
    )
    .await;
    let v = json_of(&r);
    assert_eq!(v["response_mode"], "compact");
    assert_eq!(v["sparse_transcript"], true, "{v}");
    let n_img = r.content.iter().filter(|c| c.as_image().is_some()).count();
    assert!((1..=4).contains(&n_img), "{n_img}");
    assert_eq!(v["board_images"].as_array().unwrap().len(), n_img);

    // 讲话密的视频不附图。
    let tmp2 = tempfile::TempDir::new().unwrap();
    let mut ctx2 = common::test_ctx(tmp2.path());
    ctx2.config.tools.ffmpeg_path = ff.ffmpeg.display().to_string();
    ctx2.config.tools.ffprobe_path = ff.ffprobe.display().to_string();
    let media2 = common::make_video(&ff, tmp2.path());
    let client2 = connect(AvrMcpServer::new(ctx2, common::fake_deps())).await;
    let r2 = call(
        &client2,
        "analyze_media",
        json!({ "source": media2.display().to_string() }),
    )
    .await;
    assert!(r2.content.iter().all(|c| c.as_image().is_none()));
    assert!(json_of(&r2).get("sparse_transcript").is_none());
}
