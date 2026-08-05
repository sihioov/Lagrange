"""Shared contracts of the five baseline strategy packages (plan Todo 17).

Mirrors `crates/selector/src/registry.rs` + `baseline.rs`: the lifecycle
states, the supported market/cadence/asset-class values, the promotion-gate
constants, the structured reason taxonomy (FR-SEL-005), and the
TargetPortfolio wire shape (design §6.6, Todo 16 boundary: targets only,
never orders).
"""

import hashlib
import json
from enum import Enum
from typing import Any, Dict, List, Optional

from jsonschema import ValidationError, validate as js_validate

#: The five baseline strategy ids in canonical order.
STRATEGY_IDS = [
    "buy_and_hold",
    "trend_following",
    "relative_momentum",
    "dual_momentum",
    "inverse_volatility",
]

#: Minimum Paper observation window in sessions (a calendar month of KRX
#: sessions); shorter windows are a typed denial.
MIN_PAPER_OBSERVATION_SESSIONS = 21

#: The documented Phase 3 safety evidence checklist (design §11, FR-LIVE).
PHASE3_SAFETY_CHECKS = [
    "kill_switch",
    "fail_closed_restart",
    "startup_reconciliation",
    "runtime_reconciliation",
    "credential_references",
    "rate_limits",
    "risk_gatekeeper",
    "idempotent_order_intent",
    "staged_low_value_rollout",
]


class StrategyState(str, Enum):
    DRAFT = "draft"
    VALIDATED = "validated"
    PAPER = "paper"
    LIVE_CANDIDATE = "live_candidate"
    RETIRED = "retired"

    @property
    def label(self) -> str:
        return {
            "draft": "Draft",
            "validated": "Validated",
            "paper": "Paper",
            "live_candidate": "LiveCandidate",
            "retired": "Retired",
        }[self.value]


class Market(str, Enum):
    KRX = "krx"


class Cadence(str, Enum):
    DAILY = "daily"


class AssetClass(str, Enum):
    ETF = "etf"


class TargetError(Exception):
    """A typed target-generator failure (never a panic)."""

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message


#: code -> (ko, en) with {param} placeholders; params are canonically sorted.
REASON_TEXTS: Dict[str, tuple] = {
    "SELECTED_TOP_N": (
        "상위 {top_n}개 이내 선정 (순위 {rank})",
        "Ranked {rank} within top {top_n}",
    ),
    "NOT_SELECTED_BEYOND_TOP_N": (
        "순위 {rank} — 상위 {top_n} 밖",
        "Rank {rank} is beyond top {top_n}",
    ),
    "EXCLUDED_MANDATORY_FACTOR_NULL": (
        "필수 팩터 {factor} 결측(NULL)으로 제외",
        "Excluded: mandatory factor {factor} is NULL",
    ),
    "ALL_CASH_NO_ELIGIBLE": (
        "선정 가능한 종목이 없어 전액 현금 유지",
        "No eligible instrument; portfolio held in cash",
    ),
    "WEIGHT_CAPPED_AT_MAX": (
        "최대 비중 {max_weight} 상한 적용",
        "Weight capped at max {max_weight}",
    ),
    "WEIGHT_ROUNDING_RESIDUE_TO_CASH": (
        "반올림 잔여 {residue}을 현금으로 배분",
        "Rounding residue {residue} allocated to cash",
    ),
    "CASH_FLOOR_APPLIED": (
        "현금 최소 비중 {cash_floor} 보장",
        "Cash floor {cash_floor} maintained",
    ),
    "BENCHMARK_HELD": (
        "벤치마크 {benchmark} 비중 {target_weight} 보유",
        "Holding benchmark {benchmark} at weight {target_weight}",
    ),
    "TREND_POSITIVE": (
        "빠른 이평 {fast} > 느린 이평 {slow} — 상승 추세",
        "Fast MA {fast} above slow MA {slow} — uptrend",
    ),
    "TREND_NEGATIVE_CASH": (
        "빠른 이평 {fast} <= 느린 이평 {slow} — 현금 유지",
        "Fast MA {fast} at or below slow MA {slow} — hold cash",
    ),
    "ABSOLUTE_MOMENTUM_PASSED": (
        "12개월 수익률 {return_}이 절대 모멘텀 기준 {threshold} 초과",
        "12-month return {return_} above absolute threshold {threshold}",
    ),
    "DEFENSIVE_CASH_SELECTED": (
        "최고 수익률 {return_}이 절대 모멘텀 기준 {threshold} 이하 — 방어적 현금",
        "Best return {return_} at or below absolute threshold {threshold} — defensive cash",
    ),
    "INVERSE_VOL_WEIGHTED": (
        "변동성 {vol} 역가중 비중 {weight}",
        "Inverse-volatility weight {weight} (vol {vol})",
    ),
}


def reason(code: str, **params: Any) -> Dict[str, Any]:
    """A structured evidence item: code + sorted params + ko/en text."""
    ko, en = REASON_TEXTS[code]
    sorted_params = dict(sorted(params.items()))
    return {
        "code": code,
        "params": sorted_params,
        "text_ko": ko.format(**sorted_params),
        "text_en": en.format(**sorted_params),
    }


def target_row(
    instrument_id: str,
    rank: int,
    score: float,
    factors: Dict[str, Any],
    target_weight: float,
    reasons: List[Dict[str, Any]],
) -> Dict[str, Any]:
    """One TargetRow in the Todo 16 wire shape."""
    return {
        "instrument_id": instrument_id,
        "rank": rank,
        "score": score,
        "factors": factors,
        "target_weight": target_weight,
        "reasons": reasons,
    }


def exclusion_row(instrument_id: str, reasons: List[Dict[str, Any]]) -> Dict[str, Any]:
    return {"instrument_id": instrument_id, "reasons": reasons}


def compute_portfolio_snapshot_id(portfolio: Dict[str, Any]) -> str:
    """SHA-256 over the canonical portfolio bytes (deterministic)."""
    canonical = {k: v for k, v in portfolio.items() if k != "portfolio_snapshot_id"}
    digest = hashlib.sha256(
        json.dumps(canonical, sort_keys=True, ensure_ascii=False).encode("utf-8")
    ).hexdigest()
    return f"sha256:{digest}"


def build_target_portfolio(
    *,
    as_of: str,
    strategy_version: str,
    targets: List[Dict[str, Any]],
    exclusions: List[Dict[str, Any]],
    cash_weight: float,
    constraints: Dict[str, Any],
    portfolio_reasons: List[Dict[str, Any]],
    universe_snapshot_id: str = "",
    factor_snapshot_hash: str = "",
    dataset_id: str = "",
    dataset_version: int = 0,
) -> Dict[str, Any]:
    """The TargetPortfolio wire shape (design §6.6) with its snapshot id."""
    portfolio = {
        "as_of": as_of,
        "strategy_version": strategy_version,
        "universe_snapshot_id": universe_snapshot_id,
        "factor_snapshot_hash": factor_snapshot_hash,
        "dataset_id": dataset_id,
        "dataset_version": dataset_version,
        "targets": targets,
        "exclusions": exclusions,
        "cash_weight": cash_weight,
        "constraints": constraints,
        "portfolio_reasons": portfolio_reasons,
        "portfolio_snapshot_id": "",
    }
    portfolio["portfolio_snapshot_id"] = compute_portfolio_snapshot_id(portfolio)
    return portfolio


def validate_params(parameters: Dict[str, Any], schema: Dict[str, Any]) -> None:
    """Validates generator inputs against the package's JSON Schema."""
    try:
        js_validate(parameters, schema)
    except ValidationError as error:
        raise TargetError(
            "INVALID_PARAMETERS",
            f"parameters violate the package schema: {error.message}",
        ) from error


def validate_schema_document(schema: Dict[str, Any]) -> None:
    """The schema document itself must be a valid object schema."""
    from jsonschema import Draft202012Validator

    Draft202012Validator.check_schema(schema)
    if schema.get("type") != "object":
        raise ValueError("parameter_schema.type must be 'object'")
    if not isinstance(schema.get("properties"), dict):
        raise ValueError("parameter_schema.properties must be an object")
