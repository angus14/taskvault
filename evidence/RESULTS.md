# Verified local run — 13 November 2026

`bash run.sh` completed successfully using NEAR sandbox 2.13.4 and Rust 1.97.1.

- Five contract unit tests passed: shared budget and handoff; definitive failure and retry; authorization; mutually exclusive offers; and 100 sequential spending requests with 33 accepted, 67 rejected and zero budget violations.
- One sandbox integration test passed with the compiled contract WASM and native NEAR transfers.
- In a second project within the same sandbox test, two agents submitted independent 70-milliNEAR payments concurrently against a 100-milliNEAR budget. Exactly one succeeded; the vendor's balance rose by 70.
- The deterministic selector chose an approved 70-milliNEAR vendor offer that met a 24-hour delivery rule. The sandbox checked the first vendor's balance increased by exactly 70, rejected a second 70 purchase, a duplicate, and a cheaper alternative for the already-settled purchase.
- The selector is now a standalone signed agent executable, run in separate processes for Agent A and replacement Agent C. Both check on-chain approvals before purchase. The sandbox captures their transaction hashes and verifies they match the resulting on-chain state.
- A 25-milliNEAR transfer to a missing named account failed and unlocked the purchase. The replacement agent selected another preapproved 25-milliNEAR vendor offer, which succeeded. Project spending finished at 95 milliNEAR, and the buyer withdrew the unused 5.
- `last-run.json` contains eleven transaction steps with signers, transaction hashes, project and order views, and checked vendor balance deltas. `demo/index.html` renders those snapshots; it does not generate synthetic spending states.

```text
TASKVAULT_EVIDENCE vault_funded=1000 budget=100 chosen=first first_paid=70 second_70_rejected=true duplicate_rejected=true alternative_blocked=true failed_transfer_released=true fallback_paid=25 spent=95 unused_withdrawn=5
TASKVAULT_RACE submitted=2 accepted=1 rejected=1 spent=70 budget=100
test real_near_transfers_budget_and_failure ... ok
test result: ok. 1 passed; 0 failed
```

The job story uses sequential transactions; the separate race case concurrently submits two transactions from different agent accounts. NEAR still serializes their execution at the vault. The unit tests check requests while earlier orders are reserved. These results do not establish production security assurance, every possible network schedule, multi-shard timing, or handling of a callback that never executes. The offer data and delivery estimates are buyer-supplied demo fixtures, not independently verified vendor claims. The standalone agent is rule-based, not an AI model.
