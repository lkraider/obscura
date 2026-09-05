use obscura_cdp::dispatch::{dispatch, CdpContext};
use obscura_cdp::types::{CdpRequest, CdpResponse};
use serde_json::{json, Value};

async fn cdp_raw(
    ctx: &mut CdpContext,
    id: u64,
    method: &str,
    params: Value,
    session: Option<&str>,
) -> CdpResponse {
    dispatch(
        &CdpRequest {
            id,
            method: method.to_string(),
            params,
            session_id: session.map(str::to_string),
        },
        ctx,
    )
    .await
}

async fn created_and_attached(ctx: &mut CdpContext) -> (String, String) {
    let resp = cdp_raw(
        ctx,
        100,
        "Target.createTarget",
        json!({"url": "about:blank"}),
        None,
    )
    .await;
    assert!(resp.error.is_none());
    let target_id = resp.result.unwrap()["targetId"]
        .as_str()
        .unwrap()
        .to_string();

    let resp = cdp_raw(
        ctx,
        101,
        "Target.attachToTarget",
        json!({"targetId": target_id, "flatten": true}),
        None,
    )
    .await;
    assert!(resp.error.is_none());
    let session_id = resp.result.unwrap()["sessionId"]
        .as_str()
        .unwrap()
        .to_string();

    (target_id, session_id)
}

#[tokio::test(flavor = "current_thread")]
async fn call_function_on_remote_object_validation() {
    let mut ctx = CdpContext::new();
    let (_target_id, session_id) = created_and_attached(&mut ctx).await;

    // 1. Get a valid object via Runtime.evaluate
    let eval_res = cdp_raw(
        &mut ctx,
        1,
        "Runtime.evaluate",
        json!({
            "expression": "({ foo: 42 })"
        }),
        Some(&session_id),
    )
    .await;
    assert!(eval_res.error.is_none());
    let object_id = eval_res.result.unwrap()["result"]["objectId"]
        .as_str()
        .unwrap()
        .to_string();

    // 2. callFunctionOn with valid receiver
    let call_res = cdp_raw(
        &mut ctx,
        2,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function() { return this.foo; }",
            "objectId": object_id,
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(call_res.error.is_none());
    assert_eq!(
        call_res.result.unwrap()["result"]["value"].as_f64(),
        Some(42.0)
    );

    // 3. callFunctionOn with valid argument
    let call_res2 = cdp_raw(
        &mut ctx,
        3,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function(obj) { return obj.foo; }",
            "executionContextId": 1,
            "arguments": [{"objectId": object_id}],
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(call_res2.error.is_none());
    assert_eq!(
        call_res2.result.unwrap()["result"]["value"].as_f64(),
        Some(42.0)
    );

    // 4. callFunctionOn with arbitrary invalid receiver
    let invalid_call = cdp_raw(
        &mut ctx,
        4,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function() { return this.foo; }",
            "objectId": "{\"injectedScriptId\":999,\"id\":999}",
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(invalid_call.error.is_some());
    let err_msg = invalid_call.error.unwrap().message;
    assert_eq!(err_msg, "Could not find object with given id");

    // 5. callFunctionOn with arbitrary invalid argument
    let invalid_arg_call = cdp_raw(
        &mut ctx,
        5,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function(obj) { return obj.foo; }",
            "executionContextId": 1,
            "arguments": [{"objectId": "{\"injectedScriptId\":999,\"id\":999}"}],
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(invalid_arg_call.error.is_some());
    let err_msg = invalid_arg_call.error.unwrap().message;
    assert_eq!(err_msg, "Could not find object with given id");

    // 6. Valid object receiver/argument test via DOM.getDocument / resolveNode
    // This exercises store_object_with_meta successfully registering a standard DOM handle.
    let doc_res = cdp_raw(&mut ctx, 6, "DOM.getDocument", json!({}), Some(&session_id)).await;
    let node_id = doc_res.result.unwrap()["root"]["nodeId"].as_u64().unwrap();
    let resolve_res = cdp_raw(
        &mut ctx,
        7,
        "DOM.resolveNode",
        json!({"nodeId": node_id}),
        Some(&session_id),
    )
    .await;
    let dom_obj_id = resolve_res.result.unwrap()["object"]["objectId"]
        .as_str()
        .unwrap()
        .to_string();

    let node_call_res = cdp_raw(
        &mut ctx,
        8,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function() { return this.nodeType === 9; }",
            "objectId": dom_obj_id,
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(node_call_res.error.is_none());
    assert_eq!(
        node_call_res.result.unwrap()["result"]["value"].as_bool(),
        Some(true)
    );

    let node_arg_res = cdp_raw(
        &mut ctx,
        9,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function(obj) { return obj.nodeType === 9; }",
            "executionContextId": 1,
            "arguments": [{"objectId": dom_obj_id}],
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(node_arg_res.error.is_none());
    assert_eq!(
        node_arg_res.result.unwrap()["result"]["value"].as_bool(),
        Some(true)
    );

    // 7. getProperties child handle test, including hostile key injection regression
    // The key here has quotes, newlines, and a backslash that would break simple string interpolation.
    let hostile_key = "hostile'\"\\\nkey";
    let eval_res = cdp_raw(
        &mut ctx,
        10,
        "Runtime.evaluate",
        json!({
            "expression": format!("({{ {}: {{ bar: 99 }} }})", serde_json::to_string(hostile_key).unwrap())
        }),
        Some(&session_id),
    )
    .await;
    let parent_oid = eval_res.result.unwrap()["result"]["objectId"]
        .as_str()
        .unwrap()
        .to_string();

    let props_res = cdp_raw(
        &mut ctx,
        11,
        "Runtime.getProperties",
        json!({"objectId": parent_oid}),
        Some(&session_id),
    )
    .await;
    assert!(props_res.error.is_none());
    let child_oid = props_res.result.unwrap()["result"][0]["value"]["objectId"]
        .as_str()
        .unwrap()
        .to_string();

    let child_call_res = cdp_raw(
        &mut ctx,
        12,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function() { return this.bar; }",
            "objectId": child_oid,
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(child_call_res.error.is_none());
    assert_eq!(
        child_call_res.result.unwrap()["result"]["value"].as_f64(),
        Some(99.0)
    );

    // 8. Navigate, which clears execution contexts and object store
    let nav_res = cdp_raw(
        &mut ctx,
        13,
        "Page.navigate",
        json!({
            "url": "about:blank"
        }),
        Some(&session_id),
    )
    .await;
    assert!(nav_res.error.is_none());

    // 9. Re-use previously valid ID, which should now be invalid
    let stale_call = cdp_raw(
        &mut ctx,
        14,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function() { return this.foo; }",
            "objectId": object_id,
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(stale_call.error.is_some());
    assert_eq!(
        stale_call.error.unwrap().message,
        "Could not find object with given id"
    );

    // 10. Re-use argument ID, which should now be invalid
    let stale_arg_call = cdp_raw(
        &mut ctx,
        15,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function(obj) { return obj.foo; }",
            "executionContextId": 2, // New default context id after nav
            "arguments": [{"objectId": object_id}],
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(stale_arg_call.error.is_some());
    assert_eq!(
        stale_arg_call.error.unwrap().message,
        "Could not find object with given id"
    );

    // 11. Stale DOM-resolved handle should be invalid
    let stale_dom_call = cdp_raw(
        &mut ctx,
        16,
        "Runtime.callFunctionOn",
        json!({
            "functionDeclaration": "function() { return this; }",
            "objectId": dom_obj_id,
            "returnByValue": true
        }),
        Some(&session_id),
    )
    .await;
    assert!(stale_dom_call.error.is_some());
}
