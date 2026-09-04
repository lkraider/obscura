use obscura_cdp::dispatch::{dispatch, CdpContext};
use obscura_cdp::types::CdpRequest;
use serde_json::{json, Value};

async fn cdp(
    ctx: &mut CdpContext,
    id: u64,
    method: &str,
    params: Value,
    session: Option<&str>,
) -> Result<Value, String> {
    let resp = dispatch(
        &CdpRequest {
            id,
            method: method.to_string(),
            params,
            session_id: session.map(str::to_string),
        },
        ctx,
    )
    .await;
    if let Some(err) = resp.error {
        Err(err.message)
    } else {
        Ok(resp.result.unwrap_or_else(|| json!({})))
    }
}

async fn get_eval(ctx: &mut CdpContext, id: u64, session_id: &str, expr: &str) -> Value {
    let res = cdp(
        ctx,
        id,
        "Runtime.evaluate",
        json!({
            "expression": expr,
            "returnByValue": true
        }),
        Some(session_id),
    )
    .await
    .unwrap();
    res["result"]["value"].clone()
}

#[tokio::test]
async fn test_input_insert_text() {
    let mut ctx = CdpContext::new();

    let created = cdp(
        &mut ctx,
        1,
        "Target.createTarget",
        json!({"url": "about:blank"}),
        None,
    )
    .await
    .unwrap();
    let target_id = created["targetId"].as_str().unwrap().to_string();
    let attached = cdp(
        &mut ctx,
        2,
        "Target.attachToTarget",
        json!({"targetId": target_id, "flatten": true}),
        None,
    )
    .await
    .unwrap();
    let session_id = attached["sessionId"].as_str().unwrap().to_string();

    // Rejection on missing/non-string text
    let res_missing = cdp(
        &mut ctx,
        3,
        "Input.insertText",
        json!({}),
        Some(&session_id),
    )
    .await;
    assert!(
        res_missing.is_err(),
        "Missing text parameter should be rejected"
    );

    let res_number = cdp(
        &mut ctx,
        4,
        "Input.insertText",
        json!({"text": 123}),
        Some(&session_id),
    )
    .await;
    assert!(
        res_number.is_err(),
        "Non-string text parameter should be rejected"
    );

    let setup_html = r#"
        <input type="text" id="normal">
        <input type="text" id="readonly" readonly value="ro">
        <input type="text" id="disabled" disabled value="dis">
        <textarea id="area"></textarea>
        <div id="contented" contenteditable="true"></div>
    "#;

    cdp(
        &mut ctx,
        5,
        "Runtime.evaluate",
        json!({
            "expression": format!("document.body.innerHTML = `{}`;", setup_html),
        }),
        Some(&session_id),
    )
    .await
    .unwrap();

    let setup_listeners_js = r#"
        (function() {
            window.events = [];
            function logEvent(e) {
                window.events.push({
                    type: e.type,
                    targetId: e.target.id,
                    data: e.data,
                    inputType: e.inputType
                });
            }
            document.addEventListener('beforeinput', logEvent);
            document.addEventListener('input', logEvent);
            return true;
        })()
    "#;

    cdp(
        &mut ctx,
        6,
        "Runtime.evaluate",
        json!({
            "expression": setup_listeners_js,
        }),
        Some(&session_id),
    )
    .await
    .unwrap();

    // 1. Normal input text + events + unicode
    cdp(
        &mut ctx,
        7,
        "Runtime.evaluate",
        json!({"expression": "document.getElementById('normal').focus();"}),
        Some(&session_id),
    )
    .await
    .unwrap();
    cdp(
        &mut ctx,
        8,
        "Input.insertText",
        json!({"text": "A🚀"}),
        Some(&session_id),
    )
    .await
    .unwrap();

    let val = get_eval(
        &mut ctx,
        9,
        &session_id,
        "document.getElementById('normal').value",
    )
    .await;
    assert_eq!(val, "A🚀");

    let events = get_eval(&mut ctx, 10, &session_id, "window.events").await;
    let ev_arr = events.as_array().unwrap();
    assert_eq!(ev_arr.len(), 2);
    assert_eq!(ev_arr[0]["type"], "beforeinput");
    assert_eq!(ev_arr[0]["data"], "A🚀");
    assert_eq!(ev_arr[0]["inputType"], "insertText");
    assert_eq!(ev_arr[1]["type"], "input");
    assert_eq!(ev_arr[1]["data"], "A🚀");
    assert_eq!(ev_arr[1]["inputType"], "insertText");

    // 2. Textarea + Selection Replacement
    cdp(
        &mut ctx,
        11,
        "Runtime.evaluate",
        json!({"expression": "(function() { window.events = []; var t = document.getElementById('area'); t.value = 'hello'; t.focus(); t.setSelectionRange(1, 4); return true; })()"}),
        Some(&session_id),
    )
    .await
    .unwrap();
    cdp(
        &mut ctx,
        12,
        "Input.insertText",
        json!({"text": "i"}),
        Some(&session_id),
    )
    .await
    .unwrap();

    let val_area = get_eval(
        &mut ctx,
        13,
        &session_id,
        "document.getElementById('area').value",
    )
    .await;
    assert_eq!(val_area, "hio", "Selection replacement should work");

    // 3. Readonly / Disabled
    cdp(
        &mut ctx,
        14,
        "Runtime.evaluate",
        json!({"expression": "document.getElementById('readonly').focus();"}),
        Some(&session_id),
    )
    .await
    .unwrap();
    cdp(
        &mut ctx,
        15,
        "Input.insertText",
        json!({"text": "X"}),
        Some(&session_id),
    )
    .await
    .unwrap();
    let val_ro = get_eval(
        &mut ctx,
        16,
        &session_id,
        "document.getElementById('readonly').value",
    )
    .await;
    assert_eq!(val_ro, "ro", "Readonly should not change");

    cdp(
        &mut ctx,
        17,
        "Runtime.evaluate",
        json!({"expression": "document.getElementById('disabled').focus();"}),
        Some(&session_id),
    )
    .await
    .unwrap();
    cdp(
        &mut ctx,
        18,
        "Input.insertText",
        json!({"text": "Y"}),
        Some(&session_id),
    )
    .await
    .unwrap();
    let val_dis = get_eval(
        &mut ctx,
        19,
        &session_id,
        "document.getElementById('disabled').value",
    )
    .await;
    assert_eq!(val_dis, "dis", "Disabled should not change");

    // 4. contenteditable limitation
    // We explicitly test that it currently does NOT modify contenteditable because
    // full WebIDL selection engines are bounded limitations of the current polyfill.
    cdp(
        &mut ctx,
        20,
        "Runtime.evaluate",
        json!({"expression": "document.getElementById('contented').focus();"}),
        Some(&session_id),
    )
    .await
    .unwrap();
    cdp(
        &mut ctx,
        21,
        "Input.insertText",
        json!({"text": "Z"}),
        Some(&session_id),
    )
    .await
    .unwrap();
    let val_ce = get_eval(
        &mut ctx,
        22,
        &session_id,
        "document.getElementById('contented').innerHTML",
    )
    .await;
    assert_eq!(
        val_ce, "",
        "contenteditable limitation: bounded out of scope for simplified DOM"
    );

    // 5. Empty text
    cdp(
        &mut ctx,
        23,
        "Runtime.evaluate",
        json!({"expression": "(function() { window.events = []; document.getElementById('normal').focus(); return true; })()"}),
        Some(&session_id),
    )
    .await
    .unwrap();
    cdp(
        &mut ctx,
        24,
        "Input.insertText",
        json!({"text": ""}),
        Some(&session_id),
    )
    .await
    .unwrap();
    let ev_arr2 = get_eval(&mut ctx, 25, &session_id, "window.events").await;
    assert_eq!(
        ev_arr2.as_array().unwrap().len(),
        0,
        "Empty text should not emit events"
    );
}
