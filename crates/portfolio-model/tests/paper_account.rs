//! Todo 30: Paper account opening (design §10.1 `PaperAccount`).
//!
//! A Paper account is fully described by its initial cash and cost profile.
//! Opening one must be exactly the canonical [`LedgerState`] the shared
//! ledger already implements (Todo 18) -- this suite proves the composition
//! is correct and that invalid openings are typed rejections, never a
//! silently-adjusted account.

use domain::{Currency, Money};

use portfolio_model::cost::CostProfile;
use portfolio_model::error::PortfolioError;
use portfolio_model::paper_account::NewPaperAccount;

fn krw(amount: &str) -> Money {
    Money::parse(amount, Currency::KRW).expect("valid KRW money")
}

fn krx_default() -> CostProfile {
    CostProfile::krx_etf_default().expect("default profile builds")
}

#[test]
fn paper_account_opens_with_exact_initial_cash_and_no_positions() {
    let account = NewPaperAccount::new(krw("10000000"), krx_default())
        .expect("a positive KRW deposit with the KRX default profile opens");
    let state = account.opening_state();

    assert_eq!(state.cash, krw("10000000"));
    assert_eq!(state.base_currency, Currency::KRW);
    assert!(state.positions.is_empty(), "a fresh account holds nothing");
    assert!(state.orders.is_empty());
    assert!(state.fills.is_empty());
    assert_eq!(state.last_seq, 0);
    assert_eq!(state.cost_profile, krx_default());
}

#[test]
fn paper_account_rejects_zero_initial_cash() {
    let err = NewPaperAccount::new(Money::zero(Currency::KRW), krx_default())
        .expect_err("a zero-funded account must be rejected");
    assert!(matches!(err, PortfolioError::NonPositiveInitialCash { .. }));
}

#[test]
fn paper_account_rejects_a_cost_profile_denominated_in_another_currency() {
    // custom() builds KRW money fields; initial cash in a different
    // currency must be rejected rather than silently mixing currencies.
    let profile =
        CostProfile::custom("0.0001", "1", "0", 5, "1000", "0.01").expect("a valid profile builds");
    let err = NewPaperAccount::new(Money::parse("1000000", Currency::USD).unwrap(), profile)
        .expect_err("initial cash in a currency the cost profile does not use must be rejected");
    assert!(matches!(
        err,
        PortfolioError::Domain(domain::DomainError::CurrencyMismatch { .. })
    ));
}

#[test]
fn two_accounts_opened_from_the_same_inputs_are_independent_states() {
    // Two Members opening "the same" account shape (FR-PAPER-001: cash,
    // positions, orders, fills are all forced to carry an account
    // identifier at the persistence layer; here we prove the in-memory
    // states themselves never alias).
    let a = NewPaperAccount::new(krw("10000000"), krx_default())
        .unwrap()
        .opening_state();
    let mut b = NewPaperAccount::new(krw("10000000"), krx_default())
        .unwrap()
        .opening_state();

    b.apply(portfolio_model::ledger::LedgerEvent::CashDeposit {
        seq: 1,
        amount: krw("500000"),
    })
    .expect("deposit applies");

    assert_eq!(
        a.cash,
        krw("10000000"),
        "account a is untouched by b's deposit"
    );
    assert_eq!(b.cash, krw("10500000"));
}
