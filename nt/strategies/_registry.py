"""Python mirror of the strategy registry (crates/selector/src/registry.rs).

Same contracts as the Rust side: immutable versioned packages in
`Draft | Validated | Paper | LiveCandidate | Retired`, promotion gates
(golden+holdout+cost -> Validated; parity + minimum observation window ->
Paper; Phase 3 safety evidence -> LiveCandidate), Owner-only
registration/promotion/code deployment, Member changes restricted to
schema-bound parameter configs, and an append-only audit log.
"""

import hashlib
import json
import re
from typing import Any, Dict, List, Optional, Set

from jsonschema import Draft202012Validator, ValidationError, validate as js_validate

from strategies._common import (
    MIN_PAPER_OBSERVATION_SESSIONS,
    PHASE3_SAFETY_CHECKS,
    AssetClass,
    Cadence,
    Market,
    StrategyState,
)

_STRATEGY_ID_RE = re.compile(r"^[a-z0-9_]+$")


class RegistryError(Exception):
    """A typed registry failure carrying a stable machine-readable code."""

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message


def parse_market(value: str) -> Market:
    for market in Market:
        if market.value == value:
            return market
    supported = ",".join(m.value for m in Market)
    raise RegistryError(
        "UNSUPPORTED_MARKET",
        f"unsupported market {value!r} (MVP supports {supported})",
    )


def parse_cadence(value: str) -> Cadence:
    for cadence in Cadence:
        if cadence.value == value:
            return cadence
    supported = ",".join(c.value for c in Cadence)
    raise RegistryError(
        "UNSUPPORTED_CADENCE",
        f"unsupported cadence {value!r} (MVP supports {supported})",
    )


def parse_asset_class(value: str) -> AssetClass:
    for asset_class in AssetClass:
        if asset_class.value == value:
            return asset_class
    supported = ",".join(a.value for a in AssetClass)
    raise RegistryError(
        "UNSUPPORTED_ASSET_CLASS",
        f"unsupported asset class {value!r} (MVP supports {supported})",
    )


class Actor:
    """The authenticated actor: Owner (full powers) or Member (configs only)."""

    def __init__(self, kind: str, user_id: Optional[str] = None):
        self.kind = kind
        self.user_id = user_id

    @classmethod
    def owner(cls) -> "Actor":
        return cls("owner")

    @classmethod
    def member(cls, user_id: str) -> "Actor":
        return cls("member", user_id)

    def label(self) -> str:
        return "owner" if self.kind == "owner" else f"member:{self.user_id}"

    def is_owner(self) -> bool:
        return self.kind == "owner"


class PromotionEvidence:
    """The evidence submitted for a promotion (must match the target gate)."""

    def __init__(self, kind: str, payload: Dict[str, Any]):
        self.kind = kind
        self.payload = payload

    @classmethod
    def golden(cls, golden_hash: str, holdout_hash: str, cost_hash: str) -> "PromotionEvidence":
        return cls(
            "golden",
            {
                "golden_manifest_hash": golden_hash,
                "holdout_manifest_hash": holdout_hash,
                "cost_manifest_hash": cost_hash,
            },
        )

    @classmethod
    def paper(cls, parity_id: str, sessions: int) -> "PromotionEvidence":
        return cls("paper", {"parity_report_id": parity_id, "observation_sessions": sessions})

    @classmethod
    def phase3(cls, bundle_id: str, checks: Set[str]) -> "PromotionEvidence":
        return cls("phase3", {"safety_bundle_id": bundle_id, "checks": set(checks)})


def _canonical_hash(package: Dict[str, Any]) -> str:
    """SHA-256 over the canonical definition bytes (state + hash excluded)."""
    canonical = {k: v for k, v in package.items() if k not in ("state", "canonical_hash")}
    digest = hashlib.sha256(
        json.dumps(canonical, sort_keys=True, ensure_ascii=False).encode("utf-8")
    ).hexdigest()
    return f"sha256:{digest}"


def _parse_version(value: str) -> tuple:
    """(major, minor, patch, pre) or a typed UNKNOWN_VERSION error."""
    try:
        core, _, pre = value.partition("-")
        parts = core.split(".")
        if len(parts) != 3 or any(not part.isdigit() or part != str(int(part)) for part in parts):
            raise ValueError(value)
        return (int(parts[0]), int(parts[1]), int(parts[2]), pre or None)
    except ValueError as error:
        raise RegistryError("UNKNOWN_VERSION", f"invalid semantic version {value!r}") from error


def _validate_package(package: Dict[str, Any]) -> None:
    """The documented registration contract (typed INVALID_PACKAGE denials)."""
    invalid = lambda detail: RegistryError("INVALID_PACKAGE", detail)
    strategy_id = package.get("strategy_id", "")
    if not _STRATEGY_ID_RE.fullmatch(strategy_id):
        raise invalid(f"strategy_id {strategy_id!r} must match ^[a-z0-9_]+$")
    for field in ("name", "description", "risk_description"):
        if not str(package.get(field, "")).strip():
            raise invalid(f"{field} must not be empty")
    if package.get("state") != StrategyState.DRAFT:
        raise invalid(f"packages enter the registry in Draft (got {package.get('state')})")
    if not package.get("markets"):
        raise invalid("markets must not be empty")
    if not package.get("asset_classes"):
        raise invalid("asset_classes must not be empty")
    if not package.get("cadences"):
        raise invalid("cadences must not be empty")
    if not package.get("target_generator_ref") or not package.get("nt_adapter_ref"):
        raise invalid("target_generator_ref and nt_adapter_ref must not be empty")
    for market in package["markets"]:
        parse_market(market)
    for cadence in package["cadences"]:
        parse_cadence(cadence)
    for asset_class in package["asset_classes"]:
        parse_asset_class(asset_class)
    required_factors = package.get("required_factors") or []
    for factor in required_factors:
        if not str(factor).strip():
            raise invalid("required factor ids must not be empty")
    lookback = int(package.get("minimum_lookback_sessions", 0))
    if not required_factors and lookback != 0:
        raise invalid("an empty required_factors set requires minimum_lookback_sessions == 0")
    schema = package.get("parameter_schema")
    try:
        validate_schema_document(schema)
    except ValueError as error:
        raise invalid(str(error)) from error
    try:
        js_validate(package.get("default_parameters"), schema)
    except ValidationError as error:
        raise invalid(
            f"default_parameters violate the package schema: {error.message}"
        ) from error


def validate_schema_document(schema: Any) -> None:
    Draft202012Validator.check_schema(schema)
    if schema.get("type") != "object":
        raise ValueError("parameter_schema.type must be 'object'")
    if not isinstance(schema.get("properties"), dict):
        raise ValueError("parameter_schema.properties must be an object")


class Registry:
    """Versioned immutable packages + state machine + audit + config boundary."""

    def __init__(self):
        self._packages: Dict[str, List[Dict[str, Any]]] = {}
        self._audit: List[Dict[str, Any]] = []
        self._configs: List[Dict[str, Any]] = []
        self._deployments: List[Dict[str, Any]] = []
        self._seq = 0

    def register(self, actor: Actor, package: Dict[str, Any]) -> Dict[str, Any]:
        if not actor.is_owner():
            self._deny(
                actor,
                "REGISTER",
                package.get("strategy_id"),
                str(package.get("version", "")),
                None,
                None,
                "strategy registration is Owner-only",
            )
            raise RegistryError(
                "UNAUTHORIZED",
                f"action REGISTER requires Owner; actor {actor.label()} is not Owner",
            )
        _validate_package(package)
        strategy_id = package["strategy_id"]
        version = str(package["version"])
        if any(p["version"] == version for p in self._packages.get(strategy_id, [])):
            error = RegistryError(
                "IMMUTABLE_VERSION",
                f"strategy {strategy_id} version {version} is immutable and already registered",
            )
            self._deny(actor, "REGISTER", strategy_id, version, None, None, str(error))
            raise error
        stored = {
            **package,
            "state": StrategyState.DRAFT,
            "canonical_hash": _canonical_hash(package),
        }
        self._packages.setdefault(strategy_id, []).append(stored)
        self._approve(
            actor, "REGISTER", strategy_id, version, None, None, "package version registered"
        )
        return dict(stored)

    def resolve(self, strategy_id: str, version: str) -> Dict[str, Any]:
        parsed = _parse_version(version)
        versions = self._packages.get(strategy_id)
        if versions is None:
            raise RegistryError("UNKNOWN_STRATEGY", f"strategy {strategy_id} is not registered")
        for package in versions:
            if _parse_version(str(package["version"])) == parsed:
                return dict(package)
        raise RegistryError(
            "UNKNOWN_VERSION", f"strategy {strategy_id} has no version {version}"
        )

    def resolve_latest(self, strategy_id: str) -> Dict[str, Any]:
        versions = self._packages.get(strategy_id)
        if versions is None:
            raise RegistryError("UNKNOWN_STRATEGY", f"strategy {strategy_id} is not registered")
        if not versions:
            raise RegistryError("UNKNOWN_VERSION", f"strategy {strategy_id} has no versions")
        latest = max(versions, key=lambda p: _parse_version(str(p["version"])))
        return dict(latest)

    def all_packages(self) -> List[Dict[str, Any]]:
        return [dict(p) for versions in self._packages.values() for p in versions]

    def promote(
        self,
        actor: Actor,
        strategy_id: str,
        version: str,
        to: StrategyState,
        evidence: PromotionEvidence,
    ) -> Dict[str, Any]:
        current = self.resolve(strategy_id, version)
        if not actor.is_owner():
            error = RegistryError(
                "UNAUTHORIZED",
                f"action PROMOTE requires Owner; actor {actor.label()} is not Owner",
            )
            self._deny(
                actor, "PROMOTE", strategy_id, version, current["state"], to, str(error)
            )
            raise error
        _check_gate(current["state"], to, evidence)
        self._set_state(strategy_id, version, to)
        record = {
            "strategy_id": strategy_id,
            "version": version,
            "from": current["state"],
            "to": to,
            "seq": self._seq,
        }
        self._approve(
            actor, "PROMOTE", strategy_id, version, record["from"], to,
            f"promoted to {to.label}",
        )
        return record

    def retire(self, actor: Actor, strategy_id: str, version: str) -> Dict[str, Any]:
        return self.promote(
            actor,
            strategy_id,
            version,
            StrategyState.RETIRED,
            PromotionEvidence.phase3("", set()),
        )

    def apply_member_config(
        self, actor: Actor, strategy_id: str, version: str, parameters: Dict[str, Any]
    ) -> Dict[str, Any]:
        package = self.resolve(strategy_id, version)
        try:
            js_validate(parameters, package["parameter_schema"])
        except ValidationError as error:
            registry_error = RegistryError(
                "INVALID_PARAMETERS",
                f"parameters violate the package schema: {error.message}",
            )
            self._deny(
                actor, "MEMBER_CONFIG", strategy_id, version, None, None, str(registry_error)
            )
            raise registry_error from error
        config = {
            "config_id": f"member-config-{len(self._configs) + 1}",
            "actor": actor.label(),
            "strategy_id": strategy_id,
            "strategy_version": str(package["version"]),
            "parameters": parameters,
            "seq": self._seq,
        }
        self._approve(
            actor, "MEMBER_CONFIG", strategy_id, version, None, None,
            "schema-valid member parameter config stored",
        )
        self._configs.append(dict(config))
        return config

    def deploy_code(self, actor: Actor, code: str) -> Dict[str, Any]:
        if not actor.is_owner():
            detail = (
                "strategy code deployment is Owner-only; Member changes are "
                "schema-bound parameter configs"
            )
            self._deny(actor, "DEPLOY_CODE", None, None, None, None, detail)
            raise RegistryError("MEMBER_CODE_DENIED", detail)
        deployment = {
            "deployment_id": f"deployment-{len(self._deployments) + 1}",
            "actor": actor.label(),
            "code_hash": f"sha256:{hashlib.sha256(code.encode('utf-8')).hexdigest()}",
            "seq": self._seq,
        }
        self._approve(
            actor, "DEPLOY_CODE", None, None, None, None, "Owner code deployment recorded"
        )
        self._deployments.append(dict(deployment))
        return deployment

    def audit(self) -> List[Dict[str, Any]]:
        return list(self._audit)

    def configs(self) -> List[Dict[str, Any]]:
        return list(self._configs)

    def deployments(self) -> List[Dict[str, Any]]:
        return list(self._deployments)

    def _set_state(self, strategy_id: str, version: str, to: StrategyState) -> None:
        parsed = _parse_version(version)
        for package in self._packages[strategy_id]:
            if _parse_version(str(package["version"])) == parsed:
                package["state"] = to
                return
        raise RegistryError("UNKNOWN_VERSION", f"strategy {strategy_id} has no version {version}")

    def _approve(
        self,
        actor: Actor,
        action: str,
        strategy_id: Optional[str],
        version: Optional[str],
        from_state: Optional[StrategyState],
        to_state: Optional[StrategyState],
        reason: str,
    ) -> None:
        self._push(actor, action, strategy_id, version, from_state, to_state, "APPROVED", reason)

    def _deny(
        self,
        actor: Actor,
        action: str,
        strategy_id: Optional[str],
        version: Optional[str],
        from_state: Optional[StrategyState],
        to_state: Optional[StrategyState],
        reason: str,
    ) -> None:
        self._push(actor, action, strategy_id, version, from_state, to_state, "DENIED", reason)

    def _push(
        self,
        actor: Actor,
        action: str,
        strategy_id: Optional[str],
        version: Optional[str],
        from_state: Optional[StrategyState],
        to_state: Optional[StrategyState],
        outcome: str,
        reason: str,
    ) -> None:
        self._seq += 1
        self._audit.append(
            {
                "seq": self._seq,
                "at": f"{self._seq}",
                "actor": actor.label(),
                "action": action,
                "strategy_id": strategy_id,
                "version": version,
                "from_state": from_state,
                "to_state": to_state,
                "outcome": outcome,
                "reason": reason,
            }
        )


def _check_gate(
    from_state: StrategyState,
    to_state: StrategyState,
    evidence: PromotionEvidence,
) -> None:
    """The documented transition table; typed denials on every violation."""
    invalid = lambda detail: RegistryError(
        "INVALID_PROMOTION", f"invalid promotion {from_state.label} -> {to_state.label}: {detail}"
    )
    if to_state == StrategyState.DRAFT:
        raise invalid("Draft is the entry state; promotion into Draft is not a transition")
    if from_state == StrategyState.RETIRED:
        raise invalid("Retired is terminal")
    if to_state == StrategyState.RETIRED:
        return
    if from_state == StrategyState.DRAFT and to_state == StrategyState.VALIDATED:
        _require_golden(evidence)
        return
    if from_state == StrategyState.VALIDATED and to_state == StrategyState.PAPER:
        _require_paper(evidence)
        return
    if from_state == StrategyState.PAPER and to_state == StrategyState.LIVE_CANDIDATE:
        _require_phase3(evidence)
        return
    if from_state == StrategyState.DRAFT and to_state == StrategyState.PAPER:
        raise invalid("promotion must pass through Validated (golden+holdout+cost checks)")
    if from_state == StrategyState.VALIDATED and to_state == StrategyState.LIVE_CANDIDATE:
        raise invalid("promotion must pass through Paper (parity + minimum observation window)")
    if from_state == StrategyState.DRAFT and to_state == StrategyState.LIVE_CANDIDATE:
        raise invalid("promotion must pass through Validated then Paper")
    if from_state == to_state:
        raise invalid(f"already {from_state.label}")
    raise invalid("unsupported transition")


def _require_golden(evidence: PromotionEvidence) -> None:
    if evidence.kind != "golden":
        raise RegistryError(
            "MISSING_PROMOTION_EVIDENCE",
            "promotion to Validated requires missing evidence: golden",
        )
    missing = [
        name
        for name, value in (
            ("golden", evidence.payload["golden_manifest_hash"]),
            ("holdout", evidence.payload["holdout_manifest_hash"]),
            ("cost", evidence.payload["cost_manifest_hash"]),
        )
        if not value
    ]
    if missing:
        raise RegistryError(
            "MISSING_PROMOTION_EVIDENCE",
            f"promotion to Validated requires missing evidence: {','.join(missing)}",
        )


def _require_paper(evidence: PromotionEvidence) -> None:
    if evidence.kind != "paper":
        raise RegistryError(
            "MISSING_PROMOTION_EVIDENCE",
            "promotion to Paper requires missing evidence: parity_report",
        )
    if not evidence.payload.get("parity_report_id"):
        raise RegistryError(
            "MISSING_PROMOTION_EVIDENCE",
            "promotion to Paper requires missing evidence: parity_report",
        )
    sessions = int(evidence.payload["observation_sessions"])
    if sessions < MIN_PAPER_OBSERVATION_SESSIONS:
        raise RegistryError(
            "INVALID_PROMOTION",
            f"invalid promotion Validated -> Paper: observation window {sessions} sessions "
            f"is below the minimum {MIN_PAPER_OBSERVATION_SESSIONS}",
        )


def _require_phase3(evidence: PromotionEvidence) -> None:
    if evidence.kind != "phase3":
        raise RegistryError(
            "MISSING_PROMOTION_EVIDENCE",
            "promotion to LiveCandidate requires missing evidence: phase3_safety",
        )
    missing = sorted(set(PHASE3_SAFETY_CHECKS) - set(evidence.payload.get("checks", set())))
    if missing:
        raise RegistryError(
            "MISSING_PROMOTION_EVIDENCE",
            f"promotion to LiveCandidate requires missing evidence: {','.join(missing)}",
        )


def baseline_packages() -> List[Dict[str, Any]]:
    """The five baseline packages, registered in canonical order."""
    import importlib

    packages = []
    for strategy_id in [
        "buy_and_hold",
        "trend_following",
        "relative_momentum",
        "dual_momentum",
        "inverse_volatility",
    ]:
        module = importlib.import_module(f"strategies.{strategy_id}.package")
        packages.append(dict(module.PACKAGE))
    return packages
