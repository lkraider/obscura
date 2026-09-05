#![cfg(feature = "render")]

use obscura_cdp::dispatch::{dispatch, CdpContext};
use obscura_cdp::types::CdpRequest;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

async fn serve_fixture(expected_reqs: usize) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        for _ in 0..expected_reqs {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let n = socket.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&buf);
            if request.contains("GET / ") {
                let body = r#"<!doctype html><html><body>
                    <form id="f" action="/target">
                        <input id="i" type="text" name="q" value="" oninput="if(this.value==='go') this.form.submit();">
                        <button id="b" type="submit">Submit</button>
                    </form>
                </body></html>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            } else if request.contains("GET /target") {
                let body = r#"<!doctype html><html><body>Target</body></html>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        }
    });
    (format!("http://{addr}/"), handle)
}

async fn cdp(
    ctx: &mut CdpContext,
    id: u64,
    method: &str,
    params: Value,
    session_id: &str,
) -> Value {
    let response = dispatch(
        &CdpRequest {
            id,
            method: method.to_string(),
            params,
            session_id: Some(session_id.to_string()),
        },
        ctx,
    )
    .await;
    assert!(response.error.is_none(), "CDP {method} failed: {:?}", response.error);
    response.result.unwrap_or_else(|| json!({}))
}

async fn setup(expected_reqs: usize) -> (CdpContext, String, tokio::task::JoinHandle<()>) {
    std::env::set_var("OBSCURA_ALLOW_PRIVATE_NETWORK", "1");
    let (url, handle) = serve_fixture(expected_reqs).await;
    let mut ctx = CdpContext::new();
    let page_id = ctx.create_page();
    let session_id = "test-session";
    ctx.sessions.insert(session_id.to_string(), page_id);
    cdp(
        &mut ctx,
        1,
        "Page.navigate",
        json!({"url": url}),
        session_id,
    )
    .await;
    ctx.pending_events.clear();
    (ctx, url, handle)
}

fn extract_lifecycle(ctx: &mut CdpContext) -> Vec<(String, Option<String>)> {
    let mut sequence = Vec::new();
    for ev in ctx.pending_events.drain(..) {
        if ev.method == "Runtime.executionContextsCleared" {
            sequence.push((ev.method, None));
        } else if ev.method == "Page.frameNavigated" {
            let loader = ev.params["frame"]["loaderId"].as_str().map(|s| s.to_string());
            sequence.push((ev.method, loader));
        } else if ev.method == "Page.lifecycleEvent" {
            let name = ev.params["name"].as_str().unwrap_or("").to_string();
            sequence.push((format!("{}.{}", ev.method, name), None));
        }
    }
    sequence
}

fn assert_valid_navigation_sequence(seq: &[(String, Option<String>)]) {
    let mut exec_clear_seen = false;
    let mut nav_seen = false;
    let mut load_seen = false;
    let mut init_seen = false;
    let mut loader_id = None;

    for (m, id) in seq {
        if m == "Runtime.executionContextsCleared" {
            exec_clear_seen = true;
            assert!(!nav_seen, "executionContextsCleared must precede frameNavigated");
        } else if m == "Page.frameNavigated" {
            nav_seen = true;
            assert!(exec_clear_seen, "frameNavigated must follow executionContextsCleared");
            loader_id = id.clone();
        } else if m == "Page.lifecycleEvent.init" {
            init_seen = true;
        } else if m == "Page.lifecycleEvent.load" {
            load_seen = true;
            assert!(nav_seen, "load must follow frameNavigated");
        }
    }

    assert!(init_seen, "Must contain lifecycleEvent.init");
    assert!(load_seen, "Must contain lifecycleEvent.load");

    let lid = loader_id.expect("loaderId must be present");
    assert!(!lid.is_empty(), "loaderId must not be empty");
}

#[tokio::test(flavor = "current_thread")]
async fn parity_runtime_evaluate_navigation() {
    let (mut ctx, _, handle) = setup(2).await;
    cdp(
        &mut ctx,
        2,
        "Runtime.evaluate",
        json!({"expression": "document.getElementById('b').click()", "returnByValue": true}),
        "test-session",
    )
    .await;
    let seq = extract_lifecycle(&mut ctx);
    assert_valid_navigation_sequence(&seq);
    let _ = handle.await;
}

#[tokio::test(flavor = "current_thread")]
async fn parity_input_mouse_event_navigation() {
    let (mut ctx, _, handle) = setup(2).await;
    let rect = cdp(
        &mut ctx,
        2,
        "Runtime.evaluate",
        json!({
            "expression": "document.getElementById('b').getBoundingClientRect()",
            "returnByValue": true
        }),
        "test-session",
    )
    .await;
    let x = rect["result"]["value"]["x"].as_f64().unwrap() + 5.0;
    let y = rect["result"]["value"]["y"].as_f64().unwrap() + 5.0;

    cdp(
        &mut ctx,
        3,
        "Input.dispatchMouseEvent",
        json!({"type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1}),
        "test-session",
    )
    .await;

    cdp(
        &mut ctx,
        4,
        "Input.dispatchMouseEvent",
        json!({"type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1}),
        "test-session",
    )
    .await;

    let seq = extract_lifecycle(&mut ctx);
    assert_valid_navigation_sequence(&seq);
    let _ = handle.await;
}

#[tokio::test(flavor = "current_thread")]
async fn parity_input_key_event_navigation() {
    let (mut ctx, _, handle) = setup(2).await;
    cdp(
        &mut ctx,
        2,
        "Runtime.evaluate",
        json!({"expression": "document.getElementById('i').focus()", "returnByValue": true}),
        "test-session",
    )
    .await;
    ctx.pending_events.clear();

    cdp(
        &mut ctx,
        3,
        "Input.dispatchKeyEvent",
        json!({"type": "keyDown", "key": "Enter", "code": "Enter"}),
        "test-session",
    )
    .await;

    let seq = extract_lifecycle(&mut ctx);
    assert_valid_navigation_sequence(&seq);
    let _ = handle.await;
}

#[tokio::test(flavor = "current_thread")]
async fn parity_input_insert_text_navigation() {
    let (mut ctx, _, handle) = setup(2).await;
    cdp(
        &mut ctx,
        2,
        "Runtime.evaluate",
        json!({"expression": "document.getElementById('i').focus()", "returnByValue": true}),
        "test-session",
    )
    .await;
    ctx.pending_events.clear();

    cdp(
        &mut ctx,
        3,
        "Input.insertText",
        json!({"text": "go"}),
        "test-session",
    )
    .await;

    let seq = extract_lifecycle(&mut ctx);
    assert_valid_navigation_sequence(&seq);
    let _ = handle.await;
}