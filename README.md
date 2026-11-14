# TaskVault

**Approve a job and its budget once; let agents execute its approved purchases without creating new allowances.**

A buyer funds a NEAR vault, allocates a smaller budget to a project, approves offers for named purchases with each recipient and maximum amount, and authorizes agent accounts. The on-chain contract maintains `spent + reserved <= budget` and permits only one paid offer per purchase. A replacement agent sees the same order state and cannot pay it twice.

## Reproduce the demo

On Linux or Codespaces with Git, curl, cc, Python 3 and Internet access:

```bash
bash run.sh
```

The script installs Rust 1.97.1 if necessary, verifies and starts NEAR sandbox 2.13.4, builds the contract WASM and standalone policy agent, runs unit tests and executes real NEAR sandbox transfers. It does not send funds on mainnet. A cold run can take several minutes. Look for:

```text
TASKVAULT_EVIDENCE vault_funded=1000 budget=100 chosen=first first_paid=70 second_70_rejected=true duplicate_rejected=true alternative_blocked=true failed_transfer_released=true fallback_paid=25 spent=95 unused_withdrawn=5
TASKVAULT_RACE submitted=2 accepted=1 rejected=1 spent=70 budget=100
```

The units in that line are *milliNEAR*; 1,000 = 1 NEAR. The independent [policy agent](examples/agent.rs) reads [`demo/job.json`](demo/job.json): it chooses the 70-unit offer that meets the buyer's 24-hour delivery rule over a 20-unit offer that does not. It verifies the offer against the on-chain buyer approval, signs `execute` from Agent A's account, and reports its transaction hash. The sandbox checks the vendor balance rose by exactly 70. The alternative cannot be paid even though 20 remains affordable. A review transfer to a nonexistent account definitively fails; the buyer's replacement Agent C runs the same client with the failed offer excluded, selects the approved fallback, and pays 25. Closing the job lets the owner withdraw the unused 5.

[Open the recorded job dashboard](demo/index.html). It reads [the transaction trace](evidence/last-run.json) generated from the passing sandbox run: every step includes the signer, transaction hash, on-chain project and order state, and checked vendor balance change. The two successful purchases are sent by the standalone agent executable; buyer actions and adversarial attempts are sent by the Rust scenario driver. The dashboard is an interactive replay, not a connected wallet. The video supplied separately likewise shows recorded evidence, not a live wallet.

To regenerate the 38-second captioned video from that exact trace, install Python Pillow and FFmpeg, then run `python3 scripts/render-demo.py`. It writes `TaskVault_demo.mp4` beside this repository.

The agent executable accepts `NEAR_RPC_URL`, `NEAR_VAULT_ID`, `NEAR_PROJECT_ID`, `NEAR_ACCOUNT_ID`, `NEAR_PRIVATE_KEY`, `NEAR_JOB_FILE`, `NEAR_PURCHASE_ID`, and optionally `NEAR_EXCLUDE_OFFER`. The sandbox scenario supplies these values to separate processes for Agent A and Agent C. Keep a real account's private key out of shell history and repositories; the sandbox uses its published development key. The client checks the selected offer's purchase, recipient, cap and ready status before signing. The contract remains the final authority if an agent has stale or hostile inputs.

## Buyer and agent workflow

1. Deploy `target/wasm32-unknown-unknown/release/taskvault.wasm`; call `new({"owner":"buyer.near"})`.
2. Buyer calls payable `fund()` and `create_project({"id":"job","budget":"...yoctoNEAR...","expiry_ns":...,"agents":["agent-a.near","agent-b.near"]})`.
3. Buyer calls `approve_order({"project_id":"job","purchase_id":"research-data","order_id":"vendor-a-offer","recipient":"vendor.near","max_amount":"...yoctoNEAR..."})`. Multiple offers can share the same purchase ID; only one can complete.
4. Authorized agent calls `execute({"project_id":"job","order_id":"vendor-a-offer","amount":"...yoctoNEAR..."})`. The vault reserves the amount before sending native NEAR to the approved recipient. `get_order` shows `ready`, `in_flight`, or `settled`.
5. Buyer can `replace_agent`, `close_project` when no payments are outstanding, and `withdraw_unallocated`. An agent has no method to change a recipient, increase an order cap, or create another order ID. If a transfer fails definitively, another approved offer for that purchase becomes eligible.

Only the owner may fund, allocate and approve. Agents should receive NEAR keys permitting only the vault's `execute` method; key gas allowance is separate from the on-chain purchasing budget. One vault supports multiple projects, and unallocated funds cannot be spent by any project.

## Why asynchronous accounting matters

`execute` writes `InFlight`, selects its purchase's offer, and reserves budget before issuing a NEAR transfer. The private callback moves its reservation to settled spending only after a successful transfer result. A definitive failure releases the amount and unlocks that purchase for another approved offer. A callback that never completes leaves the reservation and offer selection occupied; expiry alone never frees a possibly paid order. Attempts have increasing numbers, and stale or duplicate callbacks are rejected. The buyer can replace an agent without altering the order's identity or state.

The design currently supports **native NEAR, one account-owned vault and owner-preapproved orders**. Its `BTreeMap` state serializes the entire small demo on each call; a larger deployment needs persistent collections and storage accounting. Callback failure can leave a reservation stuck, and this prototype has no verified reconciliation path for such a case. There is no crosschain payment, oracle, AI model, production audit, or live deployment. The first over-budget case is sequential; a separate sandbox case concurrently submits two 70-unit transactions from distinct agent accounts to one 100-unit project. Exactly one succeeds, the other is rejected, and the vendor receives 70. This tests one local schedule, not all possible network schedules or multi-shard timing.

The standalone client uses a deterministic offer selector, not an AI model. The batch unit test submits 100 requests in sequence while earlier reservations remain outstanding: 33 requests of 3 units are accepted, 67 are rejected, and zero project-budget violations occur. It does not test simultaneous network receipt arrival or production throughput.

This was built for NEAR HACKATHON 2026. Suggested tracks: **AI Security & Defense**, **AI Swarms**, **Autonomous Business**. The distinction from a wallet spending limit is buyer-authorized purchases with mutually exclusive offers and payment-state continuity across agent handoffs. Reservations and deduplication themselves are known techniques; the project does not claim to invent them.

## Independent project disclosure

The same builder also submitted [RecallPay](https://github.com/angus14/recallpay) as a separate NEAR HACKATHON 2026 project. TaskVault controls a shared agent purchasing budget and handoffs. RecallPay handles paid-data corrections, dependent job pauses and prefunded refunds. The contracts, codebases and demos are independent.
