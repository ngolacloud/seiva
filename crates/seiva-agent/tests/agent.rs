//! The loop end to end over a scripted model behind the gateway: tools, approvals,
//! hooks, the bounds, cancellation, memory, and facades in-process.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::{ana, gateway, reader, PaymentsService};
use seiva_agent::{
    facades, Agent, AgentCtx, AgentError, ApproveWrites, CallOutcome, Cancel, FnTool, Hook,
    HookDecision, Limits, StopCause, Tool, ToolCall, ToolOutput,
};
use seiva_ai_gateway::{Caller, GatewayError};
use seiva_identity::Principal;
use seiva_llm::{ChatResponse, Content, StopReason, Usage};
use seiva_mcp_server::ToolSet;
use seiva_testing::llm::ScriptedModel;
use serde_json::{json, Value};

const NIF: &str = "5417123456";

fn lookup() -> Arc<dyn Tool> {
    Arc::new(FnTool::read(
        "customer_lookup",
        "Find a customer by id.",
        json!({"type": "object", "properties": {"id": {"type": "string"}}}),
        |_caller, input: Value| async move {
            Ok(format!(
                "cliente {}: NIF {NIF}",
                input["id"].as_str().unwrap_or("?")
            ))
        },
    ))
}

fn refund(log: Arc<Mutex<Vec<String>>>) -> Arc<dyn Tool> {
    Arc::new(FnTool::write(
        "refund",
        "Refund a payment.",
        json!({"type": "object"}),
        move |caller: Caller, input: Value| {
            let log = log.clone();
            async move {
                log.lock()
                    .unwrap()
                    .push(format!("{} {}", caller.principal.subject, input));
                Ok("refunded".to_owned())
            }
        },
    ))
}

fn requests(model: &ScriptedModel) -> Vec<String> {
    model
        .calls()
        .iter()
        .map(|c| serde_json::to_string(&c.request).unwrap())
        .collect()
}

#[tokio::test]
async fn the_loop_runs_a_tool_and_feeds_the_masked_labelled_result_back() {
    let model = ScriptedModel::new("cloud")
        .reply_tool_call("t1", "customer_lookup", json!({"id": "C-7"}))
        .reply_text("O cliente C-7 tem o NIF ‹TAX_ID_1›.");
    let (gw, audit) = gateway(&model);
    let agent = Agent::builder(gw, "agent")
        .system("És o assistente de suporte.")
        .tool(lookup())
        .build()
        .unwrap();
    let run = agent
        .run(&ana(), None, "Quem é o cliente C-7?", &Cancel::new())
        .await
        .unwrap();

    assert_eq!(run.stop, StopCause::Answered);
    assert_eq!(run.steps, 2);
    assert_eq!(run.answer, format!("O cliente C-7 tem o NIF {NIF}."));
    assert_eq!(run.calls[0].outcome, CallOutcome::Ok);
    assert_eq!(run.usage.output_tokens, 5 + 7);
    // The second model call got the tool result labelled untrusted, the NIF masked.
    let second = &requests(&model)[1];
    assert!(
        second.contains("untrusted_data") && second.contains("‹TAX_ID_1›"),
        "{second}"
    );
    assert!(!second.contains(NIF));
    // Every step went through the gateway's audit.
    assert_eq!(audit.entries().len(), 2);
}

#[tokio::test]
async fn a_write_does_not_run_without_an_approver_and_runs_with_one() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let script = || {
        ScriptedModel::new("cloud")
            .reply_tool_call("t1", "refund", json!({"payment": "P-1"}))
            .reply_text("feito")
    };

    let model = script();
    let (gw, _) = gateway(&model);
    let agent = Agent::builder(gw, "agent")
        .tool(refund(log.clone()))
        .build()
        .unwrap();
    let run = agent
        .run(&ana(), None, "devolve o P-1", &Cancel::new())
        .await
        .unwrap();
    assert_eq!(run.calls[0].outcome, CallOutcome::NotApproved);
    assert!(log.lock().unwrap().is_empty());
    assert!(requests(&model)[1].contains("did not approve"));

    let model = script();
    let (gw, _) = gateway(&model);
    let agent = Agent::builder(gw, "agent")
        .tool(refund(log.clone()))
        .approver(Arc::new(ApproveWrites))
        .build()
        .unwrap();
    let run = agent
        .run(&ana(), None, "devolve o P-1", &Cancel::new())
        .await
        .unwrap();
    assert_eq!(run.calls[0].outcome, CallOutcome::Ok);
    assert_eq!(
        log.lock().unwrap().clone(),
        vec!["ana {\"payment\":\"P-1\"}".to_owned()]
    );
}

struct Guard {
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl Hook for Guard {
    async fn before_tool(&self, _: &AgentCtx<'_>, call: &ToolCall) -> HookDecision {
        if call.input["id"] == "C-0" {
            HookDecision::Deny("C-0 is a restricted account".into())
        } else {
            HookDecision::Continue
        }
    }

    async fn after_tool(&self, ctx: &AgentCtx<'_>, call: &ToolCall, out: &ToolOutput) {
        self.seen
            .lock()
            .unwrap()
            .push(format!("{}:{}:{}", ctx.step, call.name, out.is_error));
    }
}

#[tokio::test]
async fn hooks_see_every_call_and_may_stop_one() {
    let model = ScriptedModel::new("cloud")
        .reply_tool_call("t1", "customer_lookup", json!({"id": "C-0"}))
        .reply_tool_call("t2", "customer_lookup", json!({"id": "C-7"}))
        .reply_text("ok");
    let (gw, _) = gateway(&model);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let agent = Agent::builder(gw, "agent")
        .tool(lookup())
        .hook(Arc::new(Guard { seen: seen.clone() }))
        .build()
        .unwrap();
    let run = agent.run(&ana(), None, "x", &Cancel::new()).await.unwrap();
    assert_eq!(
        run.calls[0].outcome,
        CallOutcome::Denied("C-0 is a restricted account".into())
    );
    assert_eq!(run.calls[1].outcome, CallOutcome::Ok);
    // A denied call never ran, so only the second reached after_tool.
    assert_eq!(
        seen.lock().unwrap().clone(),
        vec!["2:customer_lookup:false".to_owned()]
    );
}

#[tokio::test]
async fn the_step_limit_ends_a_model_that_never_stops_calling_tools() {
    let mut model = ScriptedModel::new("cloud");
    for i in 0..10 {
        model = model.reply_tool_call(format!("t{i}"), "customer_lookup", json!({"id": "C-7"}));
    }
    let (gw, _) = gateway(&model);
    let limits = Limits {
        max_steps: 3,
        ..Limits::default()
    };
    let agent = Agent::builder(gw, "agent")
        .tool(lookup())
        .limits(limits)
        .build()
        .unwrap();
    let run = agent.run(&ana(), None, "x", &Cancel::new()).await.unwrap();
    assert_eq!(run.stop, StopCause::StepLimit);
    assert_eq!(run.steps, 3);
    assert_eq!(model.calls().len(), 3);
}

#[tokio::test]
async fn the_token_budget_ends_the_run() {
    let heavy = ChatResponse::new(
        "cloud",
        vec![Content::ToolUse {
            id: "t1".into(),
            name: "customer_lookup".into(),
            input: json!({"id": "C-7"}),
        }],
        StopReason::ToolUse,
        Usage {
            input_tokens: 9_000,
            output_tokens: 2_000,
            ..Usage::default()
        },
    );
    let model = ScriptedModel::new("cloud").reply(heavy).reply_text("never");
    let (gw, _) = gateway(&model);
    let limits = Limits {
        max_tokens: 10_000,
        ..Limits::default()
    };
    let agent = Agent::builder(gw, "agent")
        .tool(lookup())
        .limits(limits)
        .build()
        .unwrap();
    let run = agent.run(&ana(), None, "x", &Cancel::new()).await.unwrap();
    assert_eq!(run.stop, StopCause::TokenLimit);
    assert_eq!(model.calls().len(), 1);
}

fn slow() -> Arc<dyn Tool> {
    Arc::new(FnTool::read(
        "slow",
        "Takes its time.",
        json!({"type": "object"}),
        |_, _| async {
            tokio::time::sleep(Duration::from_secs(600)).await;
            Ok("late".to_owned())
        },
    ))
}

#[tokio::test(start_paused = true)]
async fn a_slow_tool_times_out_and_the_run_goes_on() {
    let model = ScriptedModel::new("cloud")
        .reply_tool_call("t1", "slow", json!({}))
        .reply_text("desisti");
    let (gw, _) = gateway(&model);
    let limits = Limits {
        tool_timeout: Duration::from_secs(5),
        ..Limits::default()
    };
    let agent = Agent::builder(gw, "agent")
        .tool(slow())
        .limits(limits)
        .build()
        .unwrap();
    let run = agent.run(&ana(), None, "x", &Cancel::new()).await.unwrap();
    assert_eq!(run.calls[0].outcome, CallOutcome::TimedOut);
    assert_eq!(run.answer, "desisti");
}

#[tokio::test(start_paused = true)]
async fn a_cancel_stops_the_run_mid_tool() {
    let model = ScriptedModel::new("cloud")
        .reply_tool_call("t1", "slow", json!({}))
        .reply_text("never");
    let (gw, _) = gateway(&model);
    let agent = Agent::builder(gw, "agent").tool(slow()).build().unwrap();
    let cancel = Cancel::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        c.cancel();
    });
    let err = agent.run(&ana(), None, "x", &cancel).await.unwrap_err();
    assert_eq!(err, AgentError::Cancelled);
    assert_eq!(model.calls().len(), 1);
}

#[tokio::test]
async fn memory_continues_a_conversation_for_its_owner_only() {
    let model = ScriptedModel::new("cloud")
        .reply_text("Olá Ana.")
        .reply_text("Disseste olá.")
        .reply_text("Olá Rui.");
    let (gw, _) = gateway(&model);
    let agent = Agent::builder(gw, "agent").build().unwrap();
    agent
        .run(&ana(), Some("c1"), "olá", &Cancel::new())
        .await
        .unwrap();
    agent
        .run(&ana(), Some("c1"), "o que disse eu?", &Cancel::new())
        .await
        .unwrap();
    // Rui's "c1" is his own, empty conversation.
    agent
        .run(&reader(), Some("c1"), "olá", &Cancel::new())
        .await
        .unwrap();
    let calls = model.calls();
    assert_eq!(
        calls[1].request.messages.len(),
        3,
        "the second turn carries the first"
    );
    assert_eq!(
        calls[2].request.messages.len(),
        1,
        "another principal never loads Ana's conversation"
    );
}

#[tokio::test]
async fn a_gateway_refusal_is_the_runs_error() {
    let model = ScriptedModel::new("cloud").reply_tool_call("t1", "refund", json!({}));
    let (gw, _) = gateway(&model);
    // The model calls a tool it was not offered: the gateway blocks the answer.
    let agent = Agent::builder(gw.clone(), "agent")
        .tool(lookup())
        .build()
        .unwrap();
    let err = agent
        .run(&ana(), None, "x", &Cancel::new())
        .await
        .unwrap_err();
    assert!(
        matches!(err, AgentError::Gateway(GatewayError::Blocked { .. })),
        "{err:?}"
    );
    // A caller without the route's permission never reaches the model.
    let stranger = Caller::new(Principal::new("eve", ["agent.tools"]));
    let err = agent
        .run(&stranger, None, "x", &Cancel::new())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        AgentError::Gateway(GatewayError::Forbidden { .. })
    ));
}

#[tokio::test]
async fn facades_run_in_process_as_the_person() {
    let service = PaymentsService::default();
    let set = ToolSet::new()
        .with(common::payments_tools(service.clone()))
        .unwrap();
    let model = ScriptedModel::new("cloud")
        .reply_tool_call("t1", "payments_show", json!({"id": "P-1"}))
        .reply_tool_call(
            "t2",
            "payments_refund",
            json!({"payment": "P-1", "amount_cents": 500}),
        )
        .reply_text("feito");
    let (gw, _) = gateway(&model);
    let agent = Agent::builder(gw, "agent")
        .tools(facades::tools(&set))
        .approver(Arc::new(ApproveWrites))
        .build()
        .unwrap();
    let run = agent
        .run(&ana(), None, "mostra e devolve o P-1", &Cancel::new())
        .await
        .unwrap();
    assert_eq!(
        run.calls
            .iter()
            .map(|c| c.outcome.clone())
            .collect::<Vec<_>>(),
        vec![CallOutcome::Ok, CallOutcome::Ok]
    );
    assert!(
        requests(&model)[1].contains("\\\"seen_by\\\":\\\"ana\\\""),
        "{}",
        requests(&model)[1]
    );
    assert_eq!(
        service.refunds.lock().unwrap().clone(),
        vec![("ana".to_owned(), "P-1".to_owned(), 500)]
    );

    // Rui may only read: the facade refuses the write as an unknown tool.
    let model = ScriptedModel::new("cloud")
        .reply_tool_call(
            "t1",
            "payments_refund",
            json!({"payment": "P-2", "amount_cents": 1}),
        )
        .reply_text("não deu");
    let (gw, _) = gateway(&model);
    let agent = Agent::builder(gw, "agent")
        .tools(facades::tools(&set))
        .approver(Arc::new(ApproveWrites))
        .build()
        .unwrap();
    let run = agent
        .run(&reader(), None, "devolve o P-2", &Cancel::new())
        .await
        .unwrap();
    assert_eq!(run.calls[0].outcome, CallOutcome::Failed);
    assert_eq!(service.refunds.lock().unwrap().len(), 1);
}
