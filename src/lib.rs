use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::Serialize;
use near_sdk::{env, near, AccountId, Gas, NearToken, PanicOnDefault, Promise, PromiseError};
use std::collections::BTreeMap;

#[derive(BorshDeserialize, BorshSerialize)]
#[borsh(crate = "near_sdk::borsh")]
pub struct Project {
    budget: u128,
    spent: u128,
    reserved: u128,
    expiry_ns: u64,
    agents: Vec<AccountId>,
    closed: bool,
}

#[derive(BorshDeserialize, BorshSerialize)]
#[borsh(crate = "near_sdk::borsh")]
pub enum PaymentState {
    Ready,
    InFlight { amount: u128, attempt: u64 },
    Settled { amount: u128 },
}

#[derive(BorshDeserialize, BorshSerialize)]
#[borsh(crate = "near_sdk::borsh")]
pub struct Order {
    project_id: String,
    purchase_id: String,
    recipient: AccountId,
    max_amount: u128,
    attempts: u64,
    state: PaymentState,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct ProjectView {
    pub budget: U128,
    pub spent: U128,
    pub reserved: U128,
    pub available: U128,
    pub expiry_ns: u64,
    pub agents: Vec<AccountId>,
    pub closed: bool,
}

#[derive(Serialize)]
#[serde(crate = "near_sdk::serde")]
pub struct OrderView {
    pub project_id: String,
    pub purchase_id: String,
    pub recipient: AccountId,
    pub max_amount: U128,
    pub attempts: u64,
    pub status: String,
    pub amount: Option<U128>,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct TaskVault {
    owner: AccountId,
    unallocated: u128,
    projects: BTreeMap<String, Project>,
    orders: BTreeMap<String, Order>,
    selected_offers: BTreeMap<String, String>,
}

#[near]
impl TaskVault {
    #[init]
    pub fn new(owner: AccountId) -> Self {
        Self {
            owner,
            unallocated: 0,
            projects: BTreeMap::new(),
            orders: BTreeMap::new(),
            selected_offers: BTreeMap::new(),
        }
    }

    #[payable]
    pub fn fund(&mut self) {
        self.only_owner();
        self.unallocated = self
            .unallocated
            .checked_add(env::attached_deposit().as_yoctonear())
            .expect("overflow");
    }

    pub fn create_project(
        &mut self,
        id: String,
        budget: U128,
        expiry_ns: u64,
        agents: Vec<AccountId>,
    ) {
        self.only_owner();
        assert!(
            !id.is_empty() && !self.projects.contains_key(&id),
            "invalid or duplicate project"
        );
        assert!(
            budget.0 > 0 && budget.0 <= self.unallocated,
            "insufficient unallocated funds"
        );
        assert!(expiry_ns > env::block_timestamp(), "expired project");
        assert!(
            !agents.is_empty() && agents.len() <= 8,
            "invalid agent list"
        );
        self.unallocated -= budget.0;
        self.projects.insert(
            id,
            Project {
                budget: budget.0,
                spent: 0,
                reserved: 0,
                expiry_ns,
                agents,
                closed: false,
            },
        );
    }

    pub fn approve_order(
        &mut self,
        project_id: String,
        order_id: String,
        purchase_id: String,
        recipient: AccountId,
        max_amount: U128,
    ) {
        self.only_owner();
        let p = self.projects.get(&project_id).expect("unknown project");
        assert!(
            !p.closed && env::block_timestamp() <= p.expiry_ns,
            "project closed or expired"
        );
        assert!(
            !order_id.is_empty() && !purchase_id.is_empty() && max_amount.0 > 0,
            "invalid order"
        );
        let key = Self::key(&project_id, &order_id);
        assert!(!self.orders.contains_key(&key), "order already approved");
        self.orders.insert(
            key,
            Order {
                project_id,
                purchase_id,
                recipient,
                max_amount: max_amount.0,
                attempts: 0,
                state: PaymentState::Ready,
            },
        );
    }

    pub fn replace_agent(
        &mut self,
        project_id: String,
        old_agent: AccountId,
        new_agent: AccountId,
    ) {
        self.only_owner();
        let p = self.projects.get_mut(&project_id).expect("unknown project");
        assert!(!p.closed, "project closed");
        let slot = p
            .agents
            .iter()
            .position(|a| a == &old_agent)
            .expect("agent not found");
        p.agents[slot] = new_agent;
    }

    /// Only the preapproved recipient and maximum are used. The agent cannot authorize a new order.
    pub fn execute(&mut self, project_id: String, order_id: String, amount: U128) -> Promise {
        let key = Self::key(&project_id, &order_id);
        let p = self.projects.get_mut(&project_id).expect("unknown project");
        assert!(
            p.agents.contains(&env::predecessor_account_id()),
            "unauthorized agent"
        );
        assert!(
            !p.closed && env::block_timestamp() <= p.expiry_ns,
            "project closed or expired"
        );
        let o = self.orders.get_mut(&key).expect("order not approved");
        let purchase_key = Self::key(&project_id, &o.purchase_id);
        assert!(
            self.selected_offers
                .get(&purchase_key)
                .is_none_or(|selected| selected == &key),
            "another offer for this purchase is in flight or paid"
        );
        assert!(
            matches!(o.state, PaymentState::Ready),
            "order already in flight or paid"
        );
        assert!(
            amount.0 > 0 && amount.0 <= o.max_amount,
            "amount exceeds approved order"
        );
        assert!(
            amount.0 <= p.budget - p.spent - p.reserved,
            "project budget exceeded"
        );
        p.reserved = p.reserved.checked_add(amount.0).expect("overflow");
        o.attempts = o.attempts.checked_add(1).expect("overflow");
        let attempt = o.attempts;
        o.state = PaymentState::InFlight {
            amount: amount.0,
            attempt,
        };
        self.selected_offers.insert(purchase_key, key);
        Promise::new(o.recipient.clone())
            .transfer(NearToken::from_yoctonear(amount.0))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(20))
                    .on_payment_result(project_id, order_id, attempt),
            )
    }

    #[private]
    pub fn on_payment_result(
        &mut self,
        project_id: String,
        order_id: String,
        attempt: u64,
    ) -> bool {
        let key = Self::key(&project_id, &order_id);
        let o = self.orders.get_mut(&key).expect("missing in-flight order");
        let amount = match o.state {
            PaymentState::InFlight {
                amount,
                attempt: expected,
            } if expected == attempt => amount,
            _ => env::panic_str("stale or duplicate callback"),
        };
        let p = self.projects.get_mut(&project_id).expect("missing project");
        let purchase_key = Self::key(&project_id, &o.purchase_id);
        // A callback that never executes leaves the reservation occupied. Never free it on a timer.
        match env::promise_result_checked(0, 64) {
            Ok(_) => {
                p.reserved -= amount;
                p.spent = p.spent.checked_add(amount).expect("overflow");
                o.state = PaymentState::Settled { amount };
                true
            }
            Err(PromiseError::Failed) => {
                p.reserved -= amount;
                o.state = PaymentState::Ready;
                assert_eq!(
                    self.selected_offers.get(&purchase_key),
                    Some(&key),
                    "purchase selection mismatch"
                );
                self.selected_offers.remove(&purchase_key);
                false
            }
            Err(PromiseError::TooLong(_)) => {
                env::panic_str("unexpected transfer result; reservation retained")
            }
            Err(_) => env::panic_str("unknown result; reservation retained"),
        }
    }

    pub fn close_project(&mut self, project_id: String) {
        self.only_owner();
        let p = self.projects.get_mut(&project_id).expect("unknown project");
        assert!(
            !p.closed && p.reserved == 0,
            "project closed or payment unresolved"
        );
        self.unallocated = self
            .unallocated
            .checked_add(p.budget - p.spent)
            .expect("overflow");
        p.closed = true;
    }

    pub fn withdraw_unallocated(&mut self, amount: U128) -> Promise {
        self.only_owner();
        assert!(
            amount.0 > 0 && amount.0 <= self.unallocated,
            "insufficient unallocated funds"
        );
        self.unallocated -= amount.0;
        Promise::new(self.owner.clone())
            .transfer(NearToken::from_yoctonear(amount.0))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(Gas::from_tgas(20))
                    .on_withdraw_result(amount),
            )
    }

    #[private]
    pub fn on_withdraw_result(&mut self, amount: U128) -> bool {
        match env::promise_result_checked(0, 64) {
            Ok(_) => true,
            Err(PromiseError::Failed) => {
                self.unallocated = self.unallocated.checked_add(amount.0).expect("overflow");
                false
            }
            Err(PromiseError::TooLong(_)) => env::panic_str("unexpected transfer result"),
            Err(_) => env::panic_str("unknown withdrawal result"),
        }
    }

    pub fn get_project(&self, id: String) -> ProjectView {
        let p = self.projects.get(&id).expect("unknown project");
        ProjectView {
            budget: U128(p.budget),
            spent: U128(p.spent),
            reserved: U128(p.reserved),
            available: U128(if p.closed {
                0
            } else {
                p.budget - p.spent - p.reserved
            }),
            expiry_ns: p.expiry_ns,
            agents: p.agents.clone(),
            closed: p.closed,
        }
    }

    pub fn get_order(&self, project_id: String, order_id: String) -> OrderView {
        let o = self
            .orders
            .get(&Self::key(&project_id, &order_id))
            .expect("unknown order");
        let (status, amount) = match o.state {
            PaymentState::Ready => ("ready", None),
            PaymentState::InFlight { amount, .. } => ("in_flight", Some(U128(amount))),
            PaymentState::Settled { amount } => ("settled", Some(U128(amount))),
        };
        OrderView {
            project_id: o.project_id.clone(),
            purchase_id: o.purchase_id.clone(),
            recipient: o.recipient.clone(),
            max_amount: U128(o.max_amount),
            attempts: o.attempts,
            status: status.to_owned(),
            amount,
        }
    }

    pub fn unallocated(&self) -> U128 {
        U128(self.unallocated)
    }

    fn only_owner(&self) {
        assert_eq!(env::predecessor_account_id(), self.owner, "only owner");
    }
    fn key(project: &str, order: &str) -> String {
        format!("{}:{}:{}", project.len(), project, order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use near_sdk::test_utils::VMContextBuilder;
    use near_sdk::{testing_env, PromiseResult};

    fn acc(s: &str) -> AccountId {
        s.parse().unwrap()
    }
    fn context(caller: &str, deposit: u128) {
        let mut b = VMContextBuilder::new();
        b.current_account_id(acc("vault.test.near"))
            .predecessor_account_id(acc(caller))
            .attached_deposit(NearToken::from_yoctonear(deposit))
            .block_timestamp(1_000);
        testing_env!(b.build());
    }
    fn callback(result: PromiseResult) {
        let mut b = VMContextBuilder::new();
        b.current_account_id(acc("vault.test.near"))
            .predecessor_account_id(acc("vault.test.near"))
            .block_timestamp(2_000);
        testing_env!(
            b.build(),
            near_sdk::test_vm_config(),
            near_sdk::RuntimeFeesConfig::test(),
            Default::default(),
            vec![result]
        );
    }
    fn setup() -> TaskVault {
        context("buyer.test.near", 0);
        let mut v = TaskVault::new(acc("buyer.test.near"));
        context("buyer.test.near", 1_000);
        v.fund();
        context("buyer.test.near", 0);
        v.create_project(
            "research".into(),
            U128(100),
            1_000_000,
            vec![acc("agent-a.test.near"), acc("agent-b.test.near")],
        );
        v.approve_order(
            "research".into(),
            "data".into(),
            "dataset".into(),
            acc("vendor-a.test.near"),
            U128(70),
        );
        v.approve_order(
            "research".into(),
            "compute".into(),
            "compute".into(),
            acc("vendor-b.test.near"),
            U128(70),
        );
        v.approve_order(
            "research".into(),
            "review".into(),
            "review".into(),
            acc("vendor-c.test.near"),
            U128(25),
        );
        v
    }

    #[test]
    fn shared_budget_handoff_and_refund() {
        let mut v = setup();
        assert_eq!(v.unallocated().0, 900); // Treasury is 1000; job can spend only 100.
        context("agent-a.test.near", 0);
        let _ = v.execute("research".into(), "data".into(), U128(70));
        assert_eq!(v.get_project("research".into()).reserved.0, 70);
        context("agent-b.test.near", 0);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.execute(
                "research".into(),
                "compute".into(),
                U128(70)
            )))
            .is_err()
        );
        assert_eq!(v.get_project("research".into()).reserved.0, 70);

        context("buyer.test.near", 0);
        v.replace_agent(
            "research".into(),
            acc("agent-a.test.near"),
            acc("agent-c.test.near"),
        );
        context("agent-c.test.near", 0);
        assert_eq!(
            v.get_order("research".into(), "data".into()).status,
            "in_flight"
        );
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.execute(
                "research".into(),
                "data".into(),
                U128(70)
            )))
            .is_err()
        );
        callback(PromiseResult::Successful(vec![]));
        assert!(v.on_payment_result("research".into(), "data".into(), 1));
        assert_eq!(v.get_project("research".into()).spent.0, 70);
        assert_eq!(v.get_project("research".into()).reserved.0, 0);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.on_payment_result(
                "research".into(),
                "data".into(),
                1
            )))
            .is_err()
        );
        context("agent-c.test.near", 0);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.execute(
                "research".into(),
                "data".into(),
                U128(70)
            )))
            .is_err()
        );
        let _ = v.execute("research".into(), "review".into(), U128(25));
        callback(PromiseResult::Successful(vec![]));
        assert!(v.on_payment_result("research".into(), "review".into(), 1));
        assert_eq!(v.get_project("research".into()).spent.0, 95);
        context("buyer.test.near", 0);
        v.close_project("research".into());
        assert_eq!(v.unallocated().0, 905);
    }

    #[test]
    fn definitive_failure_releases_reservation_then_retry() {
        let mut v = setup();
        context("agent-b.test.near", 0);
        let _ = v.execute("research".into(), "compute".into(), U128(70));
        callback(PromiseResult::Failed);
        assert!(!v.on_payment_result("research".into(), "compute".into(), 1));
        assert_eq!(v.get_project("research".into()).available.0, 100);
        context("agent-b.test.near", 0);
        let _ = v.execute("research".into(), "compute".into(), U128(70));
        assert_eq!(v.get_order("research".into(), "compute".into()).attempts, 2);
        callback(PromiseResult::Successful(vec![]));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.on_payment_result(
                "research".into(),
                "compute".into(),
                1
            )))
            .is_err()
        );
        assert_eq!(v.get_project("research".into()).reserved.0, 70);
        assert!(v.on_payment_result("research".into(), "compute".into(), 2));
    }

    #[test]
    fn agents_cannot_authorize_or_redirect_orders() {
        let mut v = setup();
        context("agent-a.test.near", 0);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.approve_order(
                "research".into(),
                "fake".into(),
                "fake".into(),
                acc("attacker.test.near"),
                U128(100)
            )))
            .is_err()
        );
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.execute(
                "research".into(),
                "fake".into(),
                U128(100)
            )))
            .is_err()
        );
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.execute(
                "research".into(),
                "data".into(),
                U128(71)
            )))
            .is_err()
        );
        assert_eq!(
            v.get_order("research".into(), "data".into()).recipient,
            acc("vendor-a.test.near")
        );
    }

    #[test]
    fn one_purchase_cannot_pay_two_offers_even_when_budget_allows_both() {
        let mut v = setup();
        context("buyer.test.near", 0);
        v.approve_order(
            "research".into(),
            "data-alt".into(),
            "dataset".into(),
            acc("vendor-b.test.near"),
            U128(20),
        );
        context("agent-a.test.near", 0);
        let _ = v.execute("research".into(), "data".into(), U128(70));
        context("agent-b.test.near", 0);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.execute(
                "research".into(),
                "data-alt".into(),
                U128(20)
            )))
            .is_err()
        );
        callback(PromiseResult::Failed);
        assert!(!v.on_payment_result("research".into(), "data".into(), 1));
        context("agent-b.test.near", 0);
        let _ = v.execute("research".into(), "data-alt".into(), U128(20));
        callback(PromiseResult::Successful(vec![]));
        assert!(v.on_payment_result("research".into(), "data-alt".into(), 1));
        context("agent-a.test.near", 0);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| v.execute(
                "research".into(),
                "data".into(),
                U128(70)
            )))
            .is_err()
        );
        assert_eq!(v.get_project("research".into()).spent.0, 20);
    }

    #[test]
    fn hundred_requests_share_one_atomic_allowance() {
        let mut v = setup();
        context("buyer.test.near", 0);
        for n in 0..100 {
            let id = format!("batch-{n}");
            v.approve_order(
                "research".into(),
                id.clone(),
                id,
                acc("vendor-a.test.near"),
                U128(3),
            );
        }
        let mut accepted = 0;
        let mut rejected = 0;
        for n in 0..100 {
            context(
                if n % 2 == 0 {
                    "agent-a.test.near"
                } else {
                    "agent-b.test.near"
                },
                0,
            );
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                v.execute("research".into(), format!("batch-{n}"), U128(3))
            }));
            if result.is_ok() {
                accepted += 1;
            } else {
                rejected += 1;
            }
            let p = v.get_project("research".into());
            assert!(p.spent.0 + p.reserved.0 <= p.budget.0);
        }
        assert_eq!((accepted, rejected), (33, 67));
        for n in 0..33 {
            callback(PromiseResult::Successful(vec![]));
            assert!(v.on_payment_result("research".into(), format!("batch-{n}"), 1));
        }
        let p = v.get_project("research".into());
        assert_eq!((p.spent.0, p.reserved.0, p.available.0), (99, 0, 1));
        println!("TASKVAULT_BATCH requests=100 accepted=33 rejected=67 budget_violations=0 settled=99 available=1");
    }
}
