"""Validate paired receipt structure and calculate descriptive summaries only."""

from __future__ import annotations

import argparse
import json
import math
import re
import statistics
import sys
from pathlib import Path, PurePosixPath
from typing import Any


ARMS = ("conventional", "agent_layer")
PLANNED_PILOT_PAIRS = 12 * 3
HASH_RE = re.compile(r"^[0-9a-fA-F]{64}$")
GIT_OID_RE = re.compile(r"^(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})$")
USAGE_SOURCES = {
    "provider_billing_export",
    "provider_usage_export",
    "local_compute_meter",
}
TOP_LEVEL_FIELDS = {
    "schema_version",
    "campaign_id",
    "protocol_sha256",
    "trial_id",
    "pair_id",
    "task_id",
    "replicate",
    "arm",
    "order",
    "controls",
    "outcome",
    "independent_verification",
    "usage",
}
CONTROL_FIELDS = {
    "base_commit",
    "request_sha256",
    "acceptance_oracle_sha256",
    "agent_runtime_id",
    "model_id",
    "model_config_sha256",
    "model_budget_tokens",
    "model_budget_usd",
}
OUTCOME_FIELDS = {
    "candidate_commit",
    "merged_commit",
    "human_interventions",
    "elapsed_seconds",
    "delivery_state",
}
VERIFICATION_FIELDS = {"pre_merge", "post_merge"}
VERIFICATION_RESULT_FIELDS = {
    "base_commit",
    "checked_commit",
    "acceptance_criteria_met",
    "acceptance_criteria_total",
    "escaped_regressions",
    "executed",
    "verifier_id",
    "result",
    "run_id",
    "result_sha256",
}
USAGE_FIELDS = {
    "model_invoked",
    "input_tokens",
    "output_tokens",
    "cost_usd",
    "cost_source",
    "usage_evidence_ref",
    "usage_evidence_sha256",
}


def _object(value: Any, field: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{field} must be an object")
    return value


def _exact_fields(value: dict[str, Any], expected: set[str], field: str) -> None:
    missing = [f"{field}.{name}" for name in sorted(expected - value.keys())]
    extra = [f"{field}.{name}" for name in sorted(value.keys() - expected)]
    if missing or extra:
        details = []
        if missing:
            details.append(f"missing {', '.join(missing)}")
        if extra:
            details.append(f"unexpected {', '.join(extra)}")
        raise ValueError(f"{field}: {'; '.join(details)}")


def _text(value: Any, field: str) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ValueError(f"{field} must be a non-empty string")
    return value


def _integer(value: Any, field: str, *, minimum: int = 0) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < minimum:
        raise ValueError(f"{field} must be an integer >= {minimum}")
    return value


def _number(value: Any, field: str, *, minimum: float = 0.0) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError(f"{field} must be a finite number >= {minimum}")
    number = float(value)
    if not math.isfinite(number) or number < minimum:
        raise ValueError(f"{field} must be a finite number >= {minimum}")
    return number


def _sha256(value: Any, field: str) -> str:
    if not isinstance(value, str) or not HASH_RE.fullmatch(value):
        raise ValueError(f"{field} must be a 64-character SHA-256 hex digest")
    return value.lower()


def _git_oid(value: Any, field: str) -> str:
    if not isinstance(value, str) or not GIT_OID_RE.fullmatch(value):
        raise ValueError(f"{field} must be a 40- or 64-character Git OID")
    return value.lower()


def _evidence_ref(value: Any, field: str) -> str:
    reference = _text(value, field)
    path = PurePosixPath(reference)
    has_control = any(ord(character) < 32 or ord(character) == 127 for character in reference)
    if (
        path.is_absolute()
        or not path.parts
        or "\\" in reference
        or ":" in path.parts[0]
        or has_control
        or reference.endswith("/")
        or ".." in reference.split("/")
    ):
        raise ValueError(f"{field} must be a portable relative path inside campaign evidence")
    return reference


def _validate_verification_result(
    raw: Any,
    field: str,
    *,
    expected_commit: str,
    expected_base: str | None,
    agent_runtime_id: str,
) -> dict[str, Any]:
    result = _object(raw, field)
    _exact_fields(result, VERIFICATION_RESULT_FIELDS, field)
    if result["executed"] is not True:
        raise ValueError(f"{field}.executed must be true")
    checked_commit = _git_oid(result["checked_commit"], f"{field}.checked_commit")
    base_commit = _git_oid(result["base_commit"], f"{field}.base_commit")
    if checked_commit != expected_commit:
        raise ValueError(f"{field}.checked_commit must match the recorded commit")
    if expected_base is not None and base_commit != expected_base:
        raise ValueError(f"{field}.base_commit must match controls.base_commit")
    total = _integer(
        result["acceptance_criteria_total"], f"{field}.acceptance_criteria_total", minimum=1
    )
    met = _integer(result["acceptance_criteria_met"], f"{field}.acceptance_criteria_met")
    if met > total:
        raise ValueError(f"{field}.acceptance_criteria_met cannot exceed total")
    verdict = result["result"]
    if verdict not in ("pass", "fail", "inconclusive"):
        raise ValueError(f"{field}.result is unsupported")
    if verdict == "pass" and met != total:
        raise ValueError(f"{field}.pass requires every acceptance criterion to be met")
    verifier_id = _text(result["verifier_id"], f"{field}.verifier_id")
    if verifier_id == agent_runtime_id:
        raise ValueError(f"{field}.verifier_id must differ from controls.agent_runtime_id")
    return {
        "base_commit": base_commit,
        "checked_commit": checked_commit,
        "acceptance_criteria_met": met,
        "acceptance_criteria_total": total,
        "escaped_regressions": _integer(
            result["escaped_regressions"], f"{field}.escaped_regressions"
        ),
        "executed": True,
        "verifier_id": verifier_id,
        "result": verdict,
        "run_id": _text(result["run_id"], f"{field}.run_id"),
        "result_sha256": _sha256(result["result_sha256"], f"{field}.result_sha256"),
    }


def _validate_trial(raw: Any, index: int) -> dict[str, Any]:
    prefix = f"trial[{index}]"
    trial = _object(raw, prefix)
    _exact_fields(trial, TOP_LEVEL_FIELDS, prefix)
    if type(trial["schema_version"]) is not int or trial["schema_version"] != 1:
        raise ValueError(f"{prefix}.schema_version must be 1")

    normalized: dict[str, Any] = {"schema_version": 1}
    for name in ("campaign_id", "trial_id", "pair_id", "task_id"):
        normalized[name] = _text(trial[name], f"{prefix}.{name}")
    normalized["protocol_sha256"] = _sha256(
        trial["protocol_sha256"], f"{prefix}.protocol_sha256"
    )
    normalized["replicate"] = _integer(trial["replicate"], f"{prefix}.replicate", minimum=1)
    if trial["arm"] not in ARMS:
        raise ValueError(f"{prefix}.arm must be one of {', '.join(ARMS)}")
    normalized["arm"] = trial["arm"]
    if trial["order"] not in ("first", "second"):
        raise ValueError(f"{prefix}.order must be 'first' or 'second'")
    normalized["order"] = trial["order"]

    controls = _object(trial["controls"], f"{prefix}.controls")
    _exact_fields(controls, CONTROL_FIELDS, f"{prefix}.controls")
    base = _git_oid(controls["base_commit"], f"{prefix}.controls.base_commit")
    normalized_controls = {
        "base_commit": base,
        "request_sha256": _sha256(
            controls["request_sha256"], f"{prefix}.controls.request_sha256"
        ),
        "acceptance_oracle_sha256": _sha256(
            controls["acceptance_oracle_sha256"],
            f"{prefix}.controls.acceptance_oracle_sha256",
        ),
        "agent_runtime_id": _text(
            controls["agent_runtime_id"], f"{prefix}.controls.agent_runtime_id"
        ),
        "model_id": _text(controls["model_id"], f"{prefix}.controls.model_id"),
        "model_config_sha256": _sha256(
            controls["model_config_sha256"], f"{prefix}.controls.model_config_sha256"
        ),
        "model_budget_tokens": _integer(
            controls["model_budget_tokens"],
            f"{prefix}.controls.model_budget_tokens",
            minimum=1,
        ),
        "model_budget_usd": _number(
            controls["model_budget_usd"], f"{prefix}.controls.model_budget_usd"
        ),
    }
    normalized["controls"] = normalized_controls

    outcome = _object(trial["outcome"], f"{prefix}.outcome")
    _exact_fields(outcome, OUTCOME_FIELDS, f"{prefix}.outcome")
    state = outcome["delivery_state"]
    if state not in ("merged", "not_merged", "aborted"):
        raise ValueError(f"{prefix}.outcome.delivery_state is unsupported")
    candidate_commit = _git_oid(
        outcome["candidate_commit"], f"{prefix}.outcome.candidate_commit"
    )
    merged_value = outcome["merged_commit"]
    merged_commit = (
        _git_oid(merged_value, f"{prefix}.outcome.merged_commit")
        if merged_value is not None
        else None
    )
    if (state == "merged") != (merged_commit is not None):
        raise ValueError("outcome.delivery_state and merged_commit disagree")
    normalized["outcome"] = {
        "candidate_commit": candidate_commit,
        "merged_commit": merged_commit,
        "human_interventions": _integer(
            outcome["human_interventions"], f"{prefix}.outcome.human_interventions"
        ),
        "elapsed_seconds": _number(
            outcome["elapsed_seconds"],
            f"{prefix}.outcome.elapsed_seconds",
            minimum=0.000001,
        ),
        "delivery_state": state,
    }

    verification = _object(
        trial["independent_verification"], f"{prefix}.independent_verification"
    )
    _exact_fields(verification, VERIFICATION_FIELDS, f"{prefix}.independent_verification")
    verification_prefix = f"{prefix}.independent_verification"
    pre_merge = _validate_verification_result(
        verification["pre_merge"],
        f"{verification_prefix}.pre_merge",
        expected_commit=candidate_commit,
        expected_base=normalized_controls["base_commit"],
        agent_runtime_id=normalized_controls["agent_runtime_id"],
    )
    post_merge_raw = verification["post_merge"]
    if state == "merged":
        if post_merge_raw is None or merged_commit is None:
            raise ValueError("merged trial requires post_merge verification")
        post_merge = _validate_verification_result(
            post_merge_raw,
            f"{verification_prefix}.post_merge",
            expected_commit=merged_commit,
            expected_base=None,
            agent_runtime_id=normalized_controls["agent_runtime_id"],
        )
    else:
        if post_merge_raw is not None:
            raise ValueError("non-merged trial must not have post_merge verification")
        post_merge = None
    normalized["independent_verification"] = {
        "pre_merge": pre_merge,
        "post_merge": post_merge,
    }

    usage = _object(trial["usage"], f"{prefix}.usage")
    _exact_fields(usage, USAGE_FIELDS, f"{prefix}.usage")
    invoked = usage["model_invoked"]
    if not isinstance(invoked, bool):
        raise ValueError(f"{prefix}.usage.model_invoked must be boolean")
    input_tokens = _integer(usage["input_tokens"], f"{prefix}.usage.input_tokens")
    output_tokens = _integer(usage["output_tokens"], f"{prefix}.usage.output_tokens")
    if not invoked and (input_tokens != 0 or output_tokens != 0):
        raise ValueError("usage without a model call must report zero tokens")
    source = usage["cost_source"]
    if source not in USAGE_SOURCES:
        raise ValueError(f"{prefix}.usage.cost_source is unsupported")
    evidence_ref = _evidence_ref(
        usage["usage_evidence_ref"], f"{prefix}.usage.usage_evidence_ref"
    )
    normalized["usage"] = {
        "model_invoked": invoked,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "cost_usd": _number(usage["cost_usd"], f"{prefix}.usage.cost_usd"),
        "cost_source": source,
        "usage_evidence_ref": evidence_ref,
        "usage_evidence_sha256": _sha256(
            usage["usage_evidence_sha256"], f"{prefix}.usage.usage_evidence_sha256"
        ),
    }
    return normalized


def _median_and_mad(values: list[float | int]) -> dict[str, float | int]:
    center = statistics.median(values)
    return {
        "median": center,
        "median_absolute_deviation": statistics.median(abs(v - center) for v in values),
    }


def _effective_verification(trial: dict[str, Any]) -> dict[str, Any]:
    verification = trial["independent_verification"]
    return verification["post_merge"] or verification["pre_merge"]


def _verified_delivery(trial: dict[str, Any]) -> bool:
    post_merge = trial["independent_verification"]["post_merge"]
    return (
        trial["outcome"]["delivery_state"] == "merged"
        and post_merge is not None
        and post_merge["result"] == "pass"
    )


def _post_merge_base_drift(trial: dict[str, Any]) -> bool:
    post_merge = trial["independent_verification"]["post_merge"]
    return post_merge is not None and post_merge["base_commit"] != trial["controls"]["base_commit"]


def _merged_without_pre_merge_pass(trial: dict[str, Any]) -> bool:
    pre_merge = trial["independent_verification"]["pre_merge"]
    return trial["outcome"]["delivery_state"] == "merged" and pre_merge["result"] != "pass"


def build_report(records: list[dict[str, Any]]) -> dict[str, Any]:
    """Validate complete pairs and summarize them without supporting a gain claim."""
    if not isinstance(records, list) or not records:
        raise ValueError("at least one benchmark trial is required")

    trials = [_validate_trial(record, index) for index, record in enumerate(records)]
    trial_ids: set[str] = set()
    verifier_runs: set[str] = set()
    campaign_identity: tuple[str, str] | None = None
    campaign_model_controls: tuple[Any, ...] | None = None
    campaign_arm_runtimes: dict[str, str] = {}
    pairs: dict[str, dict[str, dict[str, Any]]] = {}
    for trial in trials:
        trial_id = trial["trial_id"]
        if trial_id in trial_ids:
            raise ValueError(f"duplicate trial_id {trial_id}")
        trial_ids.add(trial_id)
        for phase in ("pre_merge", "post_merge"):
            verification = trial["independent_verification"][phase]
            if verification is None:
                continue
            verifier_run = verification["run_id"]
            if verifier_run in verifier_runs:
                raise ValueError(f"duplicate independent verifier run_id {verifier_run}")
            verifier_runs.add(verifier_run)
        identity = (trial["campaign_id"], trial["protocol_sha256"])
        if campaign_identity is None:
            campaign_identity = identity
        elif identity != campaign_identity:
            raise ValueError("all trials must belong to one campaign and protocol")
        controls = trial["controls"]
        model_controls = tuple(
            controls[name]
            for name in (
                "model_id",
                "model_config_sha256",
                "model_budget_tokens",
                "model_budget_usd",
            )
        )
        if campaign_model_controls is None:
            campaign_model_controls = model_controls
        elif model_controls != campaign_model_controls:
            raise ValueError("model and budget controls must be consistent across the campaign")
        runtime_id = controls["agent_runtime_id"]
        prior_runtime = campaign_arm_runtimes.setdefault(trial["arm"], runtime_id)
        if prior_runtime != runtime_id:
            raise ValueError("agent runtime must be consistent within each arm")
        group = pairs.setdefault(trial["pair_id"], {})
        if trial["arm"] in group:
            raise ValueError(f"pair {trial['pair_id']} has duplicate arm {trial['arm']}")
        group[trial["arm"]] = trial

    for pair_id, group in pairs.items():
        if set(group) != set(ARMS):
            raise ValueError(f"pair {pair_id} must contain exactly one trial per arm")
        conventional = group["conventional"]
        enhanced = group["agent_layer"]
        if (
            conventional["task_id"] != enhanced["task_id"]
            or conventional["replicate"] != enhanced["replicate"]
        ):
            raise ValueError(f"pair {pair_id} task_id and replicate must match")
        shared_controls = CONTROL_FIELDS - {"agent_runtime_id"}
        if any(
            conventional["controls"][name] != enhanced["controls"][name]
            for name in shared_controls
        ):
            raise ValueError(f"pair {pair_id} controls must match within a pair")
        if conventional["order"] == enhanced["order"]:
            raise ValueError(f"pair {pair_id} must use opposite order positions")

    task_replicates: dict[str, set[int]] = {}
    pair_keys: set[tuple[str, int]] = set()
    task_material: dict[str, tuple[str, str, str]] = {}
    for pair_id, group in pairs.items():
        conventional = group["conventional"]
        task_id = conventional["task_id"]
        replicate = conventional["replicate"]
        task_replicate = (task_id, replicate)
        if task_replicate in pair_keys:
            raise ValueError(f"duplicate task_id/replicate across pairs: {task_id}/{replicate}")
        pair_keys.add(task_replicate)
        task_replicates.setdefault(task_id, set()).add(replicate)
        controls = conventional["controls"]
        material = (
            controls["base_commit"],
            controls["request_sha256"],
            controls["acceptance_oracle_sha256"],
        )
        if task_id in task_material and task_material[task_id] != material:
            raise ValueError(
                f"task_id content controls must be consistent across replicates: {task_id}"
            )
        task_material[task_id] = material

    pair_count = len(pairs)
    if pair_count == 0:
        raise ValueError("no complete comparison pairs were provided")
    order_counts = {
        "conventional_first": sum(
            group["conventional"]["order"] == "first" for group in pairs.values()
        ),
        "agent_layer_first": sum(
            group["agent_layer"]["order"] == "first" for group in pairs.values()
        ),
    }
    calibration_shape_matches = len(task_replicates) == 12 and all(
        sorted(replicates) == [1, 2, 3] for replicates in task_replicates.values()
    )
    if pair_count < PLANNED_PILOT_PAIRS:
        pilot_status = "incomplete"
    elif pair_count == PLANNED_PILOT_PAIRS:
        if not calibration_shape_matches:
            pilot_status = "complete_but_design_mismatch_not_confirmatory"
        elif abs(order_counts["conventional_first"] - order_counts["agent_layer_first"]) > 1:
            pilot_status = "complete_but_order_unbalanced_not_confirmatory"
        else:
            pilot_status = "complete_not_confirmatory"
    else:
        pilot_status = "exceeds_calibration_design_not_confirmatory"

    arm_summaries: dict[str, dict[str, Any]] = {}
    for arm in ARMS:
        arm_trials = [group[arm] for group in pairs.values()]
        assessments = [_effective_verification(item) for item in arm_trials]
        met = sum(item["acceptance_criteria_met"] for item in assessments)
        total = sum(item["acceptance_criteria_total"] for item in assessments)
        arm_summaries[arm] = {
            "trial_count": len(arm_trials),
            "acceptance_criteria_met": met,
            "acceptance_criteria_total": total,
            "acceptance_rate": met / total,
            "escaped_regressions": sum(item["escaped_regressions"] for item in assessments),
            "human_interventions": sum(
                item["outcome"]["human_interventions"] for item in arm_trials
            ),
            "elapsed_seconds": _median_and_mad(
                [item["outcome"]["elapsed_seconds"] for item in arm_trials]
            ),
            "input_tokens": sum(item["usage"]["input_tokens"] for item in arm_trials),
            "output_tokens": sum(item["usage"]["output_tokens"] for item in arm_trials),
            "cost_usd": sum(item["usage"]["cost_usd"] for item in arm_trials),
            "merged_count": sum(
                item["outcome"]["delivery_state"] == "merged" for item in arm_trials
            ),
            "verified_delivery_count": sum(_verified_delivery(item) for item in arm_trials),
            "post_merge_base_drift_count": sum(_post_merge_base_drift(item) for item in arm_trials),
            "merged_without_pre_merge_pass_count": sum(
                _merged_without_pre_merge_pass(item) for item in arm_trials
            ),
            "post_merge_failed_count": sum(
                item["independent_verification"]["post_merge"] is not None
                and item["independent_verification"]["post_merge"]["result"] == "fail"
                for item in arm_trials
            ),
            "post_merge_inconclusive_count": sum(
                item["independent_verification"]["post_merge"] is not None
                and item["independent_verification"]["post_merge"]["result"] == "inconclusive"
                for item in arm_trials
            ),
        }

    paired_metrics = {
        "acceptance_rate": lambda t: (
            _effective_verification(t)["acceptance_criteria_met"]
            / _effective_verification(t)["acceptance_criteria_total"]
        ),
        "escaped_regressions": lambda t: _effective_verification(t)["escaped_regressions"],
        "human_interventions": lambda t: t["outcome"]["human_interventions"],
        "elapsed_seconds": lambda t: t["outcome"]["elapsed_seconds"],
        "total_tokens": lambda t: t["usage"]["input_tokens"] + t["usage"]["output_tokens"],
        "cost_usd": lambda t: t["usage"]["cost_usd"],
        "verified_delivery": lambda t: int(_verified_delivery(t)),
        "post_merge_base_drift": lambda t: int(_post_merge_base_drift(t)),
        "merged_without_pre_merge_pass": lambda t: int(_merged_without_pre_merge_pass(t)),
    }
    differences: dict[str, dict[str, float | int]] = {}
    for metric, get_value in paired_metrics.items():
        deltas = [
            get_value(group["agent_layer"]) - get_value(group["conventional"])
            for group in pairs.values()
        ]
        differences[metric] = {"n_pairs": pair_count, **_median_and_mad(deltas)}

    return {
        "schema_version": 1,
        "campaign_id": campaign_identity[0] if campaign_identity else None,
        "protocol_sha256": campaign_identity[1] if campaign_identity else None,
        "observed_pairs": pair_count,
        "planned_pairs": PLANNED_PILOT_PAIRS,
        "calibration_shape": {
            "planned_unique_tasks": 12,
            "planned_repetitions_per_task": 3,
            "observed_unique_tasks": len(task_replicates),
            "observed_repetitions_per_task": {
                task_id: len(replicates) for task_id, replicates in sorted(task_replicates.items())
            },
            "matches_plan": calibration_shape_matches,
        },
        "pilot_status": pilot_status,
        "gain_claim_supported": False,
        "evidence_hashes_verified": False,
        "source_authenticity_verified": False,
        "order_counts": order_counts,
        "interpretation": (
            "descriptive paired summary only; no confirmatory claim or causal attribution"
        ),
        "provenance_limit": (
            "receipt hashes are syntax-checked but referenced artifacts and source authenticity "
            "are not verified by this calculator"
        ),
        "arm_summaries": arm_summaries,
        "paired_differences": differences,
    }


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError as exc:
            raise ValueError(f"line {line_number}: invalid JSON: {exc.msg}") from exc
    return records


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Validate paired benchmark receipts and emit descriptive summaries only."
    )
    parser.add_argument("--input", type=Path, required=True, help="UTF-8 JSONL trial receipts")
    args = parser.parse_args(argv)
    try:
        report = build_report(read_jsonl(args.input))
    except (OSError, ValueError) as exc:
        print(json.dumps({"status": "invalid", "error": str(exc)}, sort_keys=True), file=sys.stderr)
        return 2
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
