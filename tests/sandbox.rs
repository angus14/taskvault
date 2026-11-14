use near_api::{Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer};
use serde_json::{json, Value};
use std::sync::Arc;
use testresult::TestResult;

const UNIT: u128 = 1_000_000_000_000_000_000_000; // 0.001 NEAR

/// This tiny agent client filters by the buyer's delivery requirement and chooses
/// the cheapest eligible *approved* offer. It has no authority to create orders.
fn select_offer(purchase: &Value, exclude: &str) -> String {
    let deadline = purchase["latest_delivery_hours"].as_u64().unwrap();
    purchase["offers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|offer| {
            offer["delivery_hours"].as_u64().unwrap() <= deadline
                && offer["order_id"].as_str().unwrap() != exclude
        })
        .min_by_key(|offer| offer["amount_milli_near"].as_u64().unwrap())
        .expect("no eligible approved offer")["order_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Each displayed step is captured after a real sandbox transaction. The UI reads
/// these snapshots, not an independently scripted set of account balances.
async fn record(
    contract: &Contract,
    network: &NetworkConfig,
    step: &str,
    actor: &str,
    action: &str,
    outcome: &str,
    tx: &near_api::types::transaction::result::TransactionResult,
    project_id: &str,
    order_ids: &[&str],
    vendor_delta: u128,
) -> TestResult<()> {
    record_hash(contract, network, step, actor, action, outcome,
        &tx.transaction().get_hash().to_string(), project_id, order_ids, vendor_delta).await
}

async fn record_hash(
    contract: &Contract,
    network: &NetworkConfig,
    step: &str,
    actor: &str,
    action: &str,
    outcome: &str,
    tx: &str,
    project_id: &str,
    order_ids: &[&str],
    vendor_delta: u128,
) -> TestResult<()> {
    let project = view(contract, network, "get_project", json!({"id":project_id})).await?;
    let mut orders = serde_json::Map::new();
    for order_id in order_ids {
        let order = view(contract, network, "get_order", json!({"project_id":project_id,"order_id":order_id})).await?;
        orders.insert((*order_id).to_owned(), order["status"].clone());
    }
    let milli = |field: &str| -> u128 {
        project[field].as_str().unwrap().parse::<u128>().unwrap() / UNIT
    };
    println!("TASKVAULT_EVENT {}", json!({
        "step":step,"actor":actor,"action":action,"outcome":outcome,
        "project":project_id,"tx":tx,
        "budget":milli("budget"),"spent":milli("spent"),
        "reserved":milli("reserved"),"available":milli("available"),
        "closed":project["closed"],"orders":orders,"vendor_delta":vendor_delta,
    }));
    Ok(())
}

async fn run_agent(rpc: &str, account: &str, purchase: &str, exclude: &str) -> TestResult<Value> {
    let output = tokio::process::Command::new("target/debug/examples/agent")
        .env("NEAR_RPC_URL", rpc)
        .env("NEAR_VAULT_ID", "vault.sandbox")
        .env("NEAR_PROJECT_ID", "job")
        .env("NEAR_ACCOUNT_ID", account)
        .env("NEAR_PRIVATE_KEY", near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY)
        .env("NEAR_JOB_FILE", "demo/job.json")
        .env("NEAR_PURCHASE_ID", purchase)
        .env("NEAR_EXCLUDE_OFFER", exclude)
        .output().await?;
    assert!(output.status.success(), "agent failed: {}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout)?;
    let line = stdout.lines().find_map(|line| line.strip_prefix("TASKVAULT_AGENT "))
        .expect("agent did not report its signed transaction");
    let result: Value = serde_json::from_str(line)?;
    println!("TASKVAULT_AGENT {}", result);
    Ok(result)
}

async fn call(
    contract: &Contract,
    network: &NetworkConfig,
    signer: &Arc<Signer>,
    caller: &str,
    method: &str,
    args: Value,
    deposit: u128,
) -> TestResult<near_api::types::transaction::result::TransactionResult> {
    Ok(contract
        .call_function(method, args)
        .transaction()
        .deposit(NearToken::from_yoctonear(deposit))
        .gas(NearGas::from_tgas(100))
        .with_signer(caller.parse::<AccountId>()?, signer.clone())
        .send_to(network)
        .await?)
}

async fn view(
    contract: &Contract,
    network: &NetworkConfig,
    method: &str,
    args: Value,
) -> TestResult<Value> {
    Ok(contract
        .call_function(method, args)
        .read_only()
        .fetch_from(network)
        .await?
        .data)
}

#[tokio::test]
async fn real_near_transfers_budget_and_failure() -> TestResult<()> {
    let job: Value = serde_json::from_str(include_str!("../demo/job.json"))?;
    let data_offer = select_offer(&job["purchases"][0], "");
    assert_eq!(data_offer, "first"); // The 20-unit quote misses the delivery requirement.
    let first_review_offer = select_offer(&job["purchases"][1], "");
    assert_eq!(first_review_offer, "failed");
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let network = NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);
    let signer = Signer::from_secret_key(
        near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse()?,
    )?;
    for name in [
        "vault.sandbox",
        "buyer.sandbox",
        "agent-a.sandbox",
        "agent-b.sandbox",
        "agent-c.sandbox",
        "vendor.sandbox",
    ] {
        sandbox
            .create_account(name.parse()?)
            .initial_balance(NearToken::from_near(100))
            .send()
            .await?;
    }
    let wasm = std::fs::read("target/wasm32-unknown-unknown/release/taskvault.wasm")?;
    let vault = Contract("vault.sandbox".parse()?);
    Contract::deploy(vault.account_id().clone())
        .use_code(wasm)
        .with_init_call("new", json!({"owner":"buyer.sandbox"}))?
        .with_signer(signer.clone())
        .send_to(&network)
        .await?
        .assert_success();
    call(
        &vault,
        &network,
        &signer,
        "buyer.sandbox",
        "fund",
        json!({}),
        1000 * UNIT,
    )
    .await?
    .assert_success();
    call(&vault,&network,&signer,"buyer.sandbox","create_project",json!({"id":"job","budget":(100*UNIT).to_string(),"expiry_ns":u64::MAX,"agents":["agent-a.sandbox","agent-b.sandbox"]}),0).await?.assert_success();
    let mut initial = None;
    for (id, purchase, recipient, max) in [
        ("first", "data", "vendor.sandbox", 70),
        ("second", "compute", "agent-c.sandbox", 70),
        ("alternative", "data", "agent-c.sandbox", 20),
        ("failed", "failed-service", "missing.sandbox", 25),
        ("fallback", "failed-service", "vendor.sandbox", 25),
    ] {
        let approved = call(&vault,&network,&signer,"buyer.sandbox","approve_order",json!({"project_id":"job","order_id":id,"purchase_id":purchase,"recipient":recipient,"max_amount":(max*UNIT).to_string()}),0).await?;
        assert!(approved.is_success());
        initial = Some(approved);
    }
    const JOB_ORDERS: &[&str] = &["first","alternative","second","failed","fallback"];
    let initial = initial.expect("five approved offers");
    record(&vault,&network,"approved","buyer.sandbox","Fund 1,000; authorize 100 and five offers","approved",&initial,"job",JOB_ORDERS,0).await?;
    let before = Account("vendor.sandbox".parse()?)
        .view()
        .fetch_from(&network)
        .await?
        .data
        .amount;
    let first_payment = run_agent(&sandbox.rpc_addr.to_string(), "agent-a.sandbox", "data", "").await?;
    assert_eq!(first_payment["offer"], data_offer);
    assert_eq!(first_payment["status"], "settled");
    let after = Account("vendor.sandbox".parse()?)
        .view()
        .fetch_from(&network)
        .await?
        .data
        .amount;
    assert_eq!(after.as_yoctonear() - before.as_yoctonear(), 70 * UNIT);
    record_hash(&vault,&network,"first_paid","agent-a.sandbox","Select the 4-hour data offer and pay vendor","settled",first_payment["tx"].as_str().unwrap(),"job",JOB_ORDERS,70).await?;
    let rejected = call(
        &vault,
        &network,
        &signer,
        "agent-b.sandbox",
        "execute",
        json!({"project_id":"job","order_id":"second","amount":(70*UNIT).to_string()}),
        0,
    )
    .await?;
    assert!(
        !rejected.is_success(),
        "second 70 must exceed project allowance"
    );
    record(&vault,&network,"over_budget","agent-b.sandbox","Attempt another 70-unit purchase","rejected: budget",&rejected,"job",JOB_ORDERS,70).await?;
    let duplicate = call(
        &vault,
        &network,
        &signer,
        "agent-b.sandbox",
        "execute",
        json!({"project_id":"job","order_id":"first","amount":(70*UNIT).to_string()}),
        0,
    )
    .await?;
    assert!(!duplicate.is_success(), "paid order cannot pay again");
    record(&vault,&network,"duplicate","agent-b.sandbox","Retry the already-paid data order","rejected: duplicate",&duplicate,"job",JOB_ORDERS,70).await?;
    let alternative = call(
        &vault,
        &network,
        &signer,
        "agent-b.sandbox",
        "execute",
        json!({"project_id":"job","order_id":"alternative","amount":(20*UNIT).to_string()}),
        0,
    )
    .await?;
    assert!(
        !alternative.is_success(),
        "a cheaper alternative cannot pay after this purchase settled"
    );
    record(&vault,&network,"alternative","agent-b.sandbox","Try the other offer for the same purchase","rejected: already paid",&alternative,"job",JOB_ORDERS,70).await?;
    let handoff = call(
        &vault,
        &network,
        &signer,
        "buyer.sandbox",
        "replace_agent",
        json!({"project_id":"job","old_agent":"agent-a.sandbox","new_agent":"agent-c.sandbox"}),
        0,
    )
    .await?;
    assert!(handoff.is_success());
    assert_eq!(
        view(
            &vault,
            &network,
            "get_order",
            json!({"project_id":"job","order_id":"first"})
        )
        .await?["status"],
        "settled"
    );
    let old_agent = call(&vault,&network,&signer,"agent-a.sandbox","execute",json!({"project_id":"job","order_id":"second","amount":(70*UNIT).to_string()}),0).await?;
    assert!(!old_agent.is_success(), "replaced agent cannot execute");
    record(&vault,&network,"handoff","buyer.sandbox","Replace Agent A; payment history stays intact","approved",&handoff,"job",JOB_ORDERS,70).await?;
    let replacement_duplicate = call(&vault,&network,&signer,"agent-c.sandbox","execute",json!({"project_id":"job","order_id":"first","amount":(70*UNIT).to_string()}),0).await?;
    assert!(!replacement_duplicate.is_success(), "replacement cannot repay first order");
    record(&vault,&network,"handoff_retry","agent-c.sandbox","Replacement retries paid order","rejected: duplicate",&replacement_duplicate,"job",JOB_ORDERS,70).await?;
    let failed_payment = call(
        &vault,
        &network,
        &signer,
        "agent-c.sandbox",
        "execute",
        json!({"project_id":"job","order_id":first_review_offer,"amount":(25*UNIT).to_string()}),
        0,
    )
    .await?;
    assert!(failed_payment.is_success());
    assert_eq!(
        view(
            &vault,
            &network,
            "get_order",
            json!({"project_id":"job","order_id":"failed"})
        )
        .await?["status"],
        "ready"
    );
    record(&vault,&network,"transfer_failed","agent-c.sandbox","Try first approved review vendor; transfer fails","failed: reservation released",&failed_payment,"job",JOB_ORDERS,70).await?;
    let fallback_offer = select_offer(&job["purchases"][1], "failed");
    assert_eq!(fallback_offer, "fallback");
    let fallback_payment = run_agent(&sandbox.rpc_addr.to_string(), "agent-c.sandbox", "failed-service", "failed").await?;
    assert_eq!(fallback_payment["offer"], fallback_offer);
    assert_eq!(fallback_payment["status"], "settled");
    let p = view(&vault, &network, "get_project", json!({"id":"job"})).await?;
    assert_eq!(p["spent"], (95 * UNIT).to_string());
    assert_eq!(p["reserved"], "0");
    let after_fallback = Account("vendor.sandbox".parse()?).view().fetch_from(&network).await?.data.amount;
    assert_eq!(after_fallback.as_yoctonear() - before.as_yoctonear(), 95 * UNIT);
    record_hash(&vault,&network,"fallback_paid","agent-c.sandbox","Pay the approved review fallback 25","settled",fallback_payment["tx"].as_str().unwrap(),"job",JOB_ORDERS,95).await?;
    call(
        &vault,
        &network,
        &signer,
        "buyer.sandbox",
        "close_project",
        json!({"project_id":"job"}),
        0,
    )
    .await?
    .assert_success();
    assert_eq!(
        view(&vault, &network, "unallocated", json!({})).await?,
        (905 * UNIT).to_string()
    );
    let withdrawal = call(
        &vault,
        &network,
        &signer,
        "buyer.sandbox",
        "withdraw_unallocated",
        json!({"amount":(5*UNIT).to_string()}),
        0,
    )
    .await?;
    assert!(withdrawal.is_success());
    assert_eq!(
        view(&vault, &network, "unallocated", json!({})).await?,
        (900 * UNIT).to_string()
    );
    record(&vault,&network,"unused_returned","buyer.sandbox","Close job and withdraw unused 5","withdrawn",&withdrawal,"job",JOB_ORDERS,95).await?;
    // Two independent agents submit to the same funded vault concurrently.
    // NEAR serializes execution at this account: one may reserve 70, the other
    // must see only 30 left, even before the first transfer callback settles.
    call(&vault,&network,&signer,"buyer.sandbox","create_project",json!({"id":"race","budget":(100*UNIT).to_string(),"expiry_ns":u64::MAX,"agents":["agent-b.sandbox","agent-c.sandbox"]}),0).await?.assert_success();
    for (order, purchase) in [("race-a", "service-a"), ("race-b", "service-b")] {
        call(&vault,&network,&signer,"buyer.sandbox","approve_order",json!({"project_id":"race","order_id":order,"purchase_id":purchase,"recipient":"vendor.sandbox","max_amount":(70*UNIT).to_string()}),0).await?.assert_success();
    }
    let before_race = Account("vendor.sandbox".parse()?)
        .view()
        .fetch_from(&network)
        .await?
        .data
        .amount;
    let (a, b) = tokio::join!(
        call(
            &vault,
            &network,
            &signer,
            "agent-b.sandbox",
            "execute",
            json!({"project_id":"race","order_id":"race-a","amount":(70*UNIT).to_string()}),
            0
        ),
        call(
            &vault,
            &network,
            &signer,
            "agent-c.sandbox",
            "execute",
            json!({"project_id":"race","order_id":"race-b","amount":(70*UNIT).to_string()}),
            0
        )
    );
    let (a, b) = (a?, b?);
    assert_eq!(u8::from(a.is_success()) + u8::from(b.is_success()), 1);
    let race = view(&vault, &network, "get_project", json!({"id":"race"})).await?;
    assert_eq!(race["spent"], (70 * UNIT).to_string());
    assert_eq!(race["reserved"], "0");
    let after_race = Account("vendor.sandbox".parse()?)
        .view()
        .fetch_from(&network)
        .await?
        .data
        .amount;
    assert_eq!(
        after_race.as_yoctonear() - before_race.as_yoctonear(),
        70 * UNIT
    );
    let winner = if a.is_success() { &a } else { &b };
    record(&vault,&network,"race","agent-b.sandbox + agent-c.sandbox","Submit two 70-unit requests concurrently","one settled; one rejected",winner,"race",&["race-a","race-b"],70).await?;
    println!("TASKVAULT_RACE submitted=2 accepted=1 rejected=1 spent=70 budget=100");
    println!("TASKVAULT_EVIDENCE vault_funded=1000 budget=100 chosen=first first_paid=70 second_70_rejected=true duplicate_rejected=true alternative_blocked=true failed_transfer_released=true fallback_paid=25 spent=95 unused_withdrawn=5");
    Ok(())
}
