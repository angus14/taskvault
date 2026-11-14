//! Minimal independent policy agent. It selects only buyer-supplied offers,
//! verifies the on-chain approval, then signs one execute transaction.
use near_api::{AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer};
use serde_json::{json, Value};
use std::env;

const UNIT: u128 = 1_000_000_000_000_000_000_000;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let required = |name: &str| -> Result<String, Box<dyn std::error::Error>> {
        Ok(env::var(name).map_err(|_| format!("missing {name}"))?)
    };
    let job: Value = serde_json::from_str(&std::fs::read_to_string(required("NEAR_JOB_FILE")?)?)?;
    let purchase_id = required("NEAR_PURCHASE_ID")?;
    let purchase = job["purchases"].as_array().ok_or("purchases missing")?
        .iter().find(|p| p["purchase_id"] == purchase_id).ok_or("purchase missing")?;
    let deadline = purchase["latest_delivery_hours"].as_u64().ok_or("deadline missing")?;
    let excluded = env::var("NEAR_EXCLUDE_OFFER").unwrap_or_default();
    let offer = purchase["offers"].as_array().ok_or("offers missing")?.iter()
        .filter(|o| o["delivery_hours"].as_u64().is_some_and(|h| h <= deadline)
            && o["order_id"].as_str() != Some(&excluded))
        .min_by_key(|o| o["amount_milli_near"].as_u64().unwrap_or(u64::MAX))
        .ok_or("no eligible approved offer")?;
    let order_id = offer["order_id"].as_str().ok_or("order ID missing")?;
    let amount = offer["amount_milli_near"].as_u64().ok_or("amount missing")? as u128 * UNIT;
    let project = required("NEAR_PROJECT_ID")?;
    let account = required("NEAR_ACCOUNT_ID")?;
    let network = NetworkConfig::from_rpc_url("configured", required("NEAR_RPC_URL")?.parse()?);
    let contract = Contract(required("NEAR_VAULT_ID")?.parse()?);
    let approved: Value = contract.call_function("get_order", json!({"project_id":project,"order_id":order_id}))
        .read_only().fetch_from(&network).await?.data;
    if approved["purchase_id"] != purchase_id
        || approved["recipient"] != offer["vendor"]
        || approved["status"] != "ready"
        || approved["max_amount"].as_str().ok_or("on-chain cap missing")?.parse::<u128>()? < amount {
        return Err("offer differs from on-chain buyer approval".into());
    }
    let signer = Signer::from_secret_key(required("NEAR_PRIVATE_KEY")?.parse()?)?;
    let result = contract.call_function("execute", json!({"project_id":project,"order_id":order_id,"amount":amount.to_string()}))
        .transaction().deposit(NearToken::from_yoctonear(0)).gas(NearGas::from_tgas(100))
        .with_signer(account.parse::<AccountId>()?, signer).send_to(&network).await?;
    if !result.is_success() { return Err("execute transaction failed".into()); }
    let tx = result.transaction().get_hash().to_string();
    let state: Value = contract.call_function("get_order", json!({"project_id":project,"order_id":order_id}))
        .read_only().fetch_from(&network).await?.data;
    println!("TASKVAULT_AGENT {}", json!({"account":account,"project":project,
        "purchase":purchase_id,"offer":order_id,"amount_milli_near":amount/UNIT,
        "status":state["status"],"tx":tx}));
    Ok(())
}
