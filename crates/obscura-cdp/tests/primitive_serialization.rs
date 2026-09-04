use obscura_cdp::dispatch::{dispatch, CdpContext};
use obscura_cdp::types::CdpRequest;
use serde_json::{json, Value};

async fn cdp(
    ctx: &mut CdpContext,
    id: u64,
    method: &str,
    params: Value,
    session: Option<&str>,
) -> Value {
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
    assert!(resp.error.is_none(), "CDP {method} failed: {:?}", resp.error);
    resp.result.unwrap_or_else(|| json!({}))
}

async fn created_and_attached(ctx: &mut CdpContext) -> (String, String) {
    let created = cdp(ctx, 900, "Target.createTarget", json!({"url": "about:blank"}), None).await;
    let target_id = created["targetId"].as_str().unwrap().to_string();
    let attached = cdp(
        ctx,
        901,
        "Target.attachToTarget",
        json!({"targetId": target_id, "flatten": true}),
        None,
    )
    .await;
    let session_id = attached["sessionId"].as_str().unwrap().to_string();
    (target_id, session_id)
}

#[tokio::test]
async fn test_primitive_serialization() {
    let mut ctx = CdpContext::new();
    let (_target_id, session_id) = created_and_attached(&mut ctx).await;

    let test_cases = vec![
        // undefined
        ("undefined", "undefined", Value::Null),
        // null
        ("null", "object", Value::Null),
        // 0
        ("0", "number", json!(0.0)),
        // "string"
        ("'string'", "string", json!("string")),
        // false
        ("false", "boolean", json!(false)),
    ];

    let mut msg_id = 1;
    for (expr, exp_type, exp_val) in test_cases {
        let actual_expr = format!("Promise.resolve({})", expr);

        // Test Runtime.evaluate
        let res = cdp(
            &mut ctx,
            msg_id,
            "Runtime.evaluate",
            json!({
                "expression": actual_expr,
                "returnByValue": true,
                "awaitPromise": true
            }),
            Some(&session_id),
        )
        .await;
        msg_id += 1;

        let typ = res["result"]["type"].as_str().unwrap();
        assert_eq!(typ, exp_type, "evaluate({expr}): expected type {exp_type}, got {typ}");

        if typ == "undefined" {
            assert!(res["result"].get("value").is_none(), "evaluate({expr}): undefined should not have value field");
        } else if exp_val != Value::Null {
            let val = &res["result"]["value"];
            if val.is_number() && exp_val.is_number() {
                assert_eq!(val.as_f64().unwrap(), exp_val.as_f64().unwrap(), "evaluate({expr}): expected value {exp_val}, got {val}");
            } else {
                assert_eq!(val, &exp_val, "evaluate({expr}): expected value {exp_val}, got {val}");
            }
        } else if exp_type == "object" {
            // null case
            let val = &res["result"]["value"];
            assert_eq!(val, &Value::Null, "evaluate({expr}): expected null value");
            let subtype = res["result"]["subtype"].as_str().unwrap();
            assert_eq!(subtype, "null", "evaluate({expr}): expected subtype null");
        }

        // Test Runtime.callFunctionOn
        let fn_decl = format!("() => {{ return {}; }}", actual_expr);
        let res2 = cdp(
            &mut ctx,
            msg_id,
            "Runtime.callFunctionOn",
            json!({
                "functionDeclaration": fn_decl,
                "executionContextId": 1,
                "returnByValue": true,
                "awaitPromise": true
            }),
            Some(&session_id),
        )
        .await;
        msg_id += 1;

        let typ2 = res2["result"]["type"].as_str().unwrap();
        assert_eq!(typ2, exp_type, "callFunctionOn({expr}): expected type {exp_type}, got {typ2}");

        if typ2 == "undefined" {
            assert!(res2["result"].get("value").is_none(), "callFunctionOn({expr}): undefined should not have value field");
        } else if exp_val != Value::Null {
            let val2 = &res2["result"]["value"];
            if val2.is_number() && exp_val.is_number() {
                assert_eq!(val2.as_f64().unwrap(), exp_val.as_f64().unwrap(), "callFunctionOn({expr}): expected value {exp_val}, got {val2}");
            } else {
                assert_eq!(val2, &exp_val, "callFunctionOn({expr}): expected value {exp_val}, got {val2}");
            }
        } else if exp_type == "object" {
            let val2 = &res2["result"]["value"];
            assert_eq!(val2, &Value::Null, "callFunctionOn({expr}): expected null value");
            let subtype = res2["result"]["subtype"].as_str().unwrap();
            assert_eq!(subtype, "null", "callFunctionOn({expr}): expected subtype null");
        }
    }
}
